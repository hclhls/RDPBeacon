use anyhow::{Context, Result};
use std::io::Write;
use std::os::fd::AsFd;
use std::time::{Duration, Instant};
use wayland_client::globals::{registry_queue_init, GlobalList, GlobalListContents};
use wayland_client::protocol::{
    wl_buffer::{self, WlBuffer},
    wl_compositor::WlCompositor,
    wl_output::WlOutput,
    wl_region::WlRegion,
    wl_registry::WlRegistry,
    wl_shm::{self, WlShm},
    wl_shm_pool::WlShmPool,
    wl_surface::WlSurface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::viewporter::client::{
    wp_viewport::WpViewport, wp_viewporter::WpViewporter,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{Layer, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity, ZwlrLayerSurfaceV1},
};

use crate::backend::fade::{fade_alpha, FADE_STEP};

/// Returns the bytes of one little-endian ARGB8888 black pixel with the given opacity.
/// Premultiplied black always has zero colour channels, so only alpha varies.
pub fn premultiplied_black_pixel(alpha: u8) -> [u8; 4] {
    [0, 0, 0, alpha]
}

/// One black layer-shell surface covering a single output.
struct Overlay {
    surface: WlSurface,
    viewport: WpViewport,
    layer: ZwlrLayerSurfaceV1,
}

/// Dispatch state; counts configured layer surfaces and maps each one to its viewport
/// so the configure handler can stretch the 1x1 buffer to the output size.
struct State {
    configured: usize,
    viewports: Vec<(ZwlrLayerSurfaceV1, WpViewport)>,
}

/// A Wayland connection plus the globals needed to draw the fade overlay.
pub struct Session {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    compositor: WlCompositor,
    shm: WlShm,
    viewporter: WpViewporter,
    layer_shell: ZwlrLayerShellV1,
    outputs: Vec<WlOutput>,
    overlays: Vec<Overlay>,
}

impl Session {
    /// Connects and binds the required globals.
    /// Returns `Ok(None)` when the compositor lacks layer-shell or viewporter support.
    pub fn connect() -> Result<Option<Self>> {
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland display")?;
        let (globals, mut queue) =
            registry_queue_init::<State>(&conn).context("Failed to enumerate Wayland globals")?;
        let qh = queue.handle();

        let compositor = bind_global(&globals, &qh, 4..=6);
        let shm = bind_global(&globals, &qh, 1..=1);
        let viewporter = bind_global(&globals, &qh, 1..=1);
        let layer_shell = bind_global(&globals, &qh, 1..=4);
        let (Some(compositor), Some(shm), Some(viewporter), Some(layer_shell)) =
            (compositor, shm, viewporter, layer_shell)
        else {
            return Ok(None);
        };

        let outputs: Vec<WlOutput> = globals
            .contents()
            .clone_list()
            .iter()
            .filter(|g| g.interface == "wl_output")
            .map(|g| globals.registry().bind(g.name, g.version.min(4), &qh, ()))
            .collect();

        let mut state = State {
            configured: 0,
            viewports: Vec::new(),
        };
        queue
            .roundtrip(&mut state)
            .context("Wayland roundtrip failed")?;

        Ok(Some(Self {
            conn,
            queue,
            state,
            compositor,
            shm,
            viewporter,
            layer_shell,
            outputs,
            overlays: Vec::new(),
        }))
    }

    pub fn has_overlay(&self) -> bool {
        !self.overlays.is_empty()
    }

    /// Creates one overlay per output and waits until the compositor configures all of them.
    pub fn show(&mut self) -> Result<()> {
        self.destroy();
        self.state.configured = 0;
        let qh = self.queue.handle();

        for output in &self.outputs {
            let surface = self.compositor.create_surface(&qh, ());
            let viewport = self.viewporter.get_viewport(&surface, &qh, ());
            let layer = self.layer_shell.get_layer_surface(
                &surface,
                Some(output),
                Layer::Overlay,
                "rdpbeacon-fade".into(),
                &qh,
                (),
            );
            layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
            layer.set_exclusive_zone(-1);
            layer.set_size(0, 0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);

            // An empty input region makes the overlay click-through.
            let region = self.compositor.create_region(&qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();

            self.state.viewports.push((layer.clone(), viewport.clone()));
            surface.commit();
            self.overlays.push(Overlay {
                surface,
                viewport,
                layer,
            });
        }

        if self.overlays.is_empty() {
            anyhow::bail!("No Wayland outputs found for the fade overlay");
        }

        for _ in 0..10 {
            self.queue.roundtrip(&mut self.state)?;
            if self.state.configured >= self.overlays.len() {
                return Ok(());
            }
        }
        self.destroy();
        anyhow::bail!("Compositor did not configure the fade overlay")
    }

    /// Attaches a fresh 1x1 black buffer with the given opacity to every overlay.
    fn set_alpha(&mut self, alpha: u8) -> Result<()> {
        let qh = self.queue.handle();
        let mut file = std::fs::File::from(
            rustix::fs::memfd_create("rdpbeacon-fade", rustix::fs::MemfdFlags::CLOEXEC)
                .context("memfd_create failed")?,
        );
        file.write_all(&premultiplied_black_pixel(alpha))
            .context("Failed to write fade pixel")?;

        let pool = self.shm.create_pool(file.as_fd(), 4, &qh, ());
        let buffer = pool.create_buffer(0, 1, 1, 4, wl_shm::Format::Argb8888, &qh, ());
        // The buffer stays valid after the pool is destroyed.
        pool.destroy();

        for o in &self.overlays {
            o.surface.attach(Some(&buffer), 0, 0);
            o.surface.damage_buffer(0, 0, 1, 1);
            o.surface.commit();
        }
        self.conn
            .flush()
            .context("Failed to flush Wayland connection")?;
        self.queue.roundtrip(&mut self.state)?;
        Ok(())
    }

    /// Animates opacity over `duration`: 0 -> 255 when `fading_out`, else 255 -> 0.
    pub fn animate(&mut self, duration: Duration, fading_out: bool) -> Result<()> {
        let start = Instant::now();
        loop {
            let elapsed = start.elapsed();
            let alpha = fade_alpha(elapsed, duration);
            self.set_alpha(if fading_out { alpha } else { 255 - alpha })?;
            if elapsed >= duration {
                return Ok(());
            }
            std::thread::sleep(FADE_STEP);
        }
    }

    /// Destroys all overlays. Safe to call repeatedly.
    pub fn destroy(&mut self) {
        for o in self.overlays.drain(..) {
            o.viewport.destroy();
            o.layer.destroy();
            o.surface.destroy();
        }
        self.state.viewports.clear();
        let _ = self.conn.flush();
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.destroy();
    }
}

fn bind_global<I>(
    globals: &GlobalList,
    qh: &QueueHandle<State>,
    versions: std::ops::RangeInclusive<u32>,
) -> Option<I>
where
    I: Proxy + 'static,
    State: Dispatch<I, ()>,
{
    globals.bind(qh, versions, ()).ok()
}

impl Dispatch<WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for State {
    fn event(
        state: &mut Self,
        layer: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_layer_surface_v1::Event::Configure {
            serial,
            width,
            height,
        } = event
        {
            layer.ack_configure(serial);
            if let Some((_, viewport)) = state.viewports.iter().find(|(l, _)| l == layer) {
                viewport.set_destination(width as i32, height as i32);
            }
            state.configured += 1;
        }
    }
}

impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _: &mut Self,
        buffer: &WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            buffer.destroy();
        }
    }
}

delegate_noop!(State: WlCompositor);
delegate_noop!(State: WlShm);
delegate_noop!(State: WlShmPool);
delegate_noop!(State: WlSurface);
delegate_noop!(State: WlRegion);
delegate_noop!(State: ignore WlOutput);
delegate_noop!(State: WpViewporter);
delegate_noop!(State: WpViewport);
delegate_noop!(State: ZwlrLayerShellV1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_pixel_is_all_zero() {
        assert_eq!(premultiplied_black_pixel(0), [0, 0, 0, 0]);
    }

    #[test]
    fn opaque_pixel_is_black_with_full_alpha() {
        assert_eq!(premultiplied_black_pixel(255), [0, 0, 0, 255]);
    }

    #[test]
    fn midpoint_keeps_colour_channels_zero() {
        assert_eq!(premultiplied_black_pixel(128), [0, 0, 0, 128]);
    }
}
