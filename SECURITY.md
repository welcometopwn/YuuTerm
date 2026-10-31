# Security

## Reporting a vulnerability

Please don't open a public issue for security problems. Use **Report a
vulnerability** on the Security tab of this repository, or email
welcometopwn@outlook.com.

Tell me the version, how to reproduce it and what an attacker could do with it.
Leave out real passwords, keys and saved sessions. Only the latest preview gets
fixes, and this is a one-person project, so I can't promise a response time.

## Known limitations

- **RSA key files:** the Rust `rsa` crate used for SSH is affected by
  [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html), a
  timing side channel. There is no fixed version yet. If you can, use an Ed25519
  key or the Windows OpenSSH agent instead of an RSA key file.
- **RDP:** YuuTerm uses the Remote Desktop control built into Windows. When you
  remember a server certificate, YuuTerm checks it before your password is sent
  and blocks the connection if it changed. This check works with TLS up to 1.2;
  if a server needs anything else, YuuTerm refuses instead of connecting
  unchecked.
- **Telnet** is not encrypted. **Serial** connections don't authenticate the
  device on the other end.
- **X11 forwarding** starts a bundled X server that only listens locally and
  uses a new cookie each time. Forwarded programs are trusted, not sandboxed.
- Testing covered a limited set of servers, devices and Windows setups.

## Privacy

YuuTerm has no telemetry and doesn't check for updates. It only connects or
transfers files when you tell it to. Windows itself and the WebView2 installer
may talk to Microsoft under their own terms.

Terminal logging is off by default; if you turn it on, logs can contain
sensitive output. Clipboard sharing is set per connection type. Session exports
contain hosts and settings, never passwords or trusted host keys. Saved
passwords stay in the Windows Credential Manager.
