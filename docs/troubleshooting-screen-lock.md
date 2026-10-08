# Troubleshooting screen-lock timeouts

## Scope and findings

This guide covers Windows 10 Enterprise locked **inside Horizon**, while the
local client desktop stays unlocked. Locking the local workstation is a separate
case: its lock screen can intercept input before Horizon receives it.

RDPBeacon reads idle time on the local machine, locates a local Horizon window,
focuses it where supported, and injects local keyboard or mouse events. It does
not inspect the remote Windows lock state, query Horizon Agent's idle timer, or
receive confirmation that the guest accepted input. A `Beacon sent successfully`
message confirms only that the local backend returned successfully.

The local X11 backend and Horizon window detection were verified during this
investigation. The remote locked-session timeout has not been reproduced or its
effective policies inspected. Input not updating the remote idle timer is a
hypothesis, not a confirmed cause. A server-enforced lifetime limit or a
disconnect/logoff rule can also explain the symptom.

## Identify what actually expires

Record the lock time, disconnect time, exact error text, Horizon Client and Agent
versions, and whether applications survive reconnection. Use the same session
and configuration for an unlocked baseline and a locked test.

| Observation | What to investigate |
| --- | --- |
| Windows shows its sign-in screen, but Horizon stays connected | Screen lock alone; not evidence of a disconnected session |
| Connection closes after a repeatable period without remote input | Horizon Agent idle policy; RDS idle policy if applicable |
| Connection closes after a fixed time since Horizon login, even with activity | Horizon's forced session lifetime limit |
| Applications disappear after reconnecting | Logoff after disconnect, pool behavior, or another logoff policy |
| Connection closes immediately upon lock | Agent/client logs and policies or scripts triggered by locking |

## Solution: retain the session while Windows stays locked

Have the desktop administrator inspect the effective settings for the affected
VM and pool. Keep password-protected screen locking enabled.

For a Horizon VDI desktop, check the Horizon Agent ADMX settings under
`View Agent Configuration > Agent Configuration` (labels vary by version):

- **Idle Time Until Disconnect (VDI)**: choose an idle retention period that
  covers the required locked interval, or `Never` when indefinite retention is
  intended. Disabled or unconfigured also means no disconnect from this policy,
  unless another applicable policy imposes a limit.
- **Disconnect Session Time Limit (VDI)**: choose how long a disconnected
  session should retain its applications before logoff.
- Check the pool's **Automatically logoff after disconnect** setting as well.
  The Agent disconnect-session GPO takes precedence over this pool setting.

Reconnect after the Agent policy update; these timeout changes take effect on
the next connection. If Dynamic Environment Manager Horizon Smart Policies are
used, inspect their idle-disconnect settings too.

Also inspect **Forcibly disconnect users** under Horizon Console's
`Settings > Global Settings > General Settings`. That limit is measured from
Horizon login and is independent of beacon activity. The similarly named
**Horizon Console Idle Session Timeout** governs the administrator console,
not the remote desktop. **Disconnect Applications and Discard SSO credentials
for Idle Users** targets application sessions; it does not disconnect existing
desktop sessions.

If native RDP/RDS session limits apply to the connection, inspect
`Administrative Templates > Windows Components > Remote Desktop Services >
Remote Desktop Session Host > Session Time Limits` in computer and user policy:

- `Set time limit for active but idle Remote Desktop Services sessions`
- `Set time limit for active Remote Desktop Services sessions`
- `Set time limit for disconnected sessions`
- `End session when time limits are reached`

Do not assume an RDS setting alone controls a Horizon Blast or PCoIP desktop.
Identify the enforcing component from its effective policy and disconnect logs.

## Collect evidence without changing policies

On the remote Windows machine, use an elevated Command Prompt to create an
effective computer-policy report; create the user report as the affected user:

```bat
gpresult /scope computer /h "%TEMP%\rdpbeacon-computer-policy.html"
gpresult /scope user /h "%TEMP%\rdpbeacon-user-policy.html"
```

Inspect the reports for the settings above. Missing Horizon settings in a
report do not establish that no Agent, pool, or Smart Policy timeout exists.
Ask the administrator to correlate Horizon Agent and Connection Server logs
with the recorded disconnect time.

For a short local diagnostic run, create a temporary configuration:

```toml
interval = "30s"
jitter = "0s"
idle_threshold = "0s"
mode = "key"
key = "Shift_L"
```

```bash
rdpbeacon --config /tmp/rdpbeacon-lock-test.toml -v run
```

Compare unlocked and remote-locked runs over the original timeout period. This
removes scheduling and local idle skipping as variables. The default interval
can reach 260 seconds with jitter, which is too long for shorter idle limits.

If key beacons work unlocked but fail locked, repeat with `mode = "mouse"` and
the pointer over the Horizon desktop surface. Mouse events can reach the window
under the pointer rather than the keyboard-focused window. This is a diagnostic
comparison, not a guaranteed solution for locked-session retention. Confirm
connection survival beyond the original timeout before treating it as effective.

When server settings cannot be changed, RDPBeacon has no verified client-only
method to guarantee retention of this locked session. Increasing pulse counts
or running a normal input simulator inside the guest does not establish such a
guarantee. Preserve work with the administrator's disconnected-session retention
settings and reconnect when needed.

## Verification after policy changes

1. Reconnect and verify the updated effective settings.
2. Stop RDPBeacon, lock only the remote Windows desktop, and wait beyond the
   previous timeout while the local machine remains awake and connected.
3. Confirm that the connection remains open and Windows still requires credentials.
4. If disconnected-session retention was changed, separately test reconnecting
   after a disconnect and verify that applications remain running.

## Official references

- [Omnissa: Configure Desktop Session Timeouts](https://docs.omnissa.com/WindowsDesktops-and-Applications-in-Horizon-V2303/ConfigureDesktopSessionTimeoutsinHorizonConsole)
- [Omnissa: Global Settings for Client and Console Sessions](https://docs.omnissa.com/Horizon-Administration-V2306/GlobalSettingsforClientandConsoleSessions)
- [Omnissa: Horizon Smart Policies for Computer Environment Settings](https://docs.omnissa.com/DEMAdminGuide-V2406/ConfigureHorizonSmartPoliciesforComputerEnvironmentSettings)
- [Microsoft: Troubleshoot unexpected RDS session locks or disconnections](https://learn.microsoft.com/en-us/troubleshoot/windows-server/remote/troubleshoot-unexpected-rds-session-locks-or-disconnections)
