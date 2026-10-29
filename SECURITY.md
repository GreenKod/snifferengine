# Security Policy

## Supported Versions

Security updates and vulnerability remediation are actively maintained for the following versions:

| Version / Branch | Supported          |
| ---------------- | ------------------ |
| `master`         | Yes                |
| Latest Release   | Yes                |
| Older Versions   | No                 |

---

## Reporting a Vulnerability

If you discover a security vulnerability in SnifferEngine, please report it responsibly so it can be addressed safely.

### Private Disclosure Process

**Do not report potential security vulnerabilities through public GitHub issues or discussions.**

If you discover a vulnerability—especially one involving:
- QuickJS sandbox escape or arbitrary native code execution in `sniffer_plugin`,
- Host Bridge authorization or capability permission gating bypass,
- Unchecked native dynamic package loading (C ABI boundary bypass in `sniffer_pkg`),
- Memory corruption, out-of-bounds GPU buffer access, or worker thread deadlock via malformed UI AST payloads,

Please submit a private report via GitHub Security Advisories:
[Submit a Security Advisory](https://github.com/GreenKod/snifferengine/security/advisories/new)

### What to Include in Your Report

To help reproduce and fix the issue quickly, please provide:
1. A clear description of the vulnerability and its potential impact.
2. Affected crates (`sniffer_core`, `sniffer_render`, `sniffer_plugin`, `sniffer_pkg`, `sniffer_platform_android`, `sniffer_platform_desktop`, or `framework/`).
3. Step-by-step reproduction instructions or a minimal proof-of-concept test case.
4. Any suggested fix, if available.

---

## Security Architecture & Threat Model

SnifferEngine enforces defense-in-depth across multiple system boundaries:

1. **JavaScript Plugin Sandbox (`sniffer_plugin`)**:
   - QuickJS runtimes execute with a hard per-plugin memory quota (`maxMemoryMb`, clamped to `2..=64` MiB) and an atomic watchdog deadline (`JS_TICK_DEADLINE_MS = 500ms`, `init = 2000ms`).
   - Plugin and framework scripts must match their manifest-declared SHA-256 digests prior to evaluation.
2. **Host Bridge Capability Gating**:
   - Host platform operations enforce caller identification (`caller_id`) and verify manifest permission grants via `permission_manager`.
   - Administrative host operations are restricted strictly to master plugins (`is_master == true`).
3. **Native Dynamic Packages (`sniffer_pkg` / ADR 001)**:
   - Shared libraries (`.so` / `.dll`) must adhere to the versioned C ABI contract (`"SNIF"` magic, `abi_version == 1`).
   - Binaries are validated against cryptographic SHA-256 hashes before dynamic linking (`dlopen` / `LoadLibrary`).
4. **Supply-Chain Quarantine**:
   - Automated CI gates (`check_dependency_age.py` and `cargo-deny`) enforce a 24-hour publication quarantine on new `crates.io` releases and audit dependency licenses and security advisories.

