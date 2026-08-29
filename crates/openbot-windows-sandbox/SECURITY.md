# Windows Engine boundary security notes

Owner: `openbot-computer::engine`; review gate: v4 R119/R127 and P1 Windows conformance.

This crate is the sole workspace exception to `unsafe_code = deny`. It exists because the
first-source ten-crate rule explicitly permits a crate for an independent security boundary and
the safe standard library cannot create a restricted-token process or authenticate a Named Pipe
peer.

The Windows implementation may call only these Win32 mechanisms:

- token: `OpenProcessToken`, `CreateRestrictedToken`, `SetTokenInformation`, `IsTokenRestricted`;
- process identity/lifecycle: `CreateProcessAsUserW`, `GetProcessTimes`, `OpenProcess`, wait/exit,
  terminate/resume, and `CloseHandle`;
- confinement: Job Object create/configure/terminate, low-integrity directory ACL/label;
- IPC: one-instance, local-only, overlapped Named Pipe plus `GetNamedPipeClientProcessId`;
- packaging: transactional PE resource update and read-only resource verification;
- allocation/conversion needed to build SIDs and security descriptors.

Invariants:

1. No raw handle or pointer crosses the public API.
2. The restricted child is created suspended and attached to a preconfigured Job before resume.
3. Breakaway flags are never enabled; Job close kills the whole process tree.
4. Handle inheritance is an explicit three-handle allowlist (stdin plus null stdout/stderr).
5. The token disables maximum privileges, is a LUA token, stays medium integrity, and uses
   `WRITE_RESTRICTED` with the Restricted Code SID; its default DACL includes that SID.
6. Profile/temp directories have a protected current-user/System/Restricted-Code DACL and an
   inheritable low label; unrelated medium objects do not grant the restricting SID.
7. Named Pipes reject remote clients, are non-inheritable, random, current-user-only, and low-label.
8. Peer acceptance requires both exact spawned PID and exact 100 ns creation FILETIME.
9. Any setup failure closes owned handles; a post-create failure terminates and reaps the child.

The boundary is `Degraded`, not `Enforced`: proxy arguments and a write-restricted token do not
constitute a Windows network or executable-path allowlist and do not resist another malicious
process running as the same user. P1 remains red until the Windows real-machine conformance test
proves Electron compatibility, renderer sandbox state, process limits, cleanup, and negative
controls. Static cross-compilation is not runtime evidence.
