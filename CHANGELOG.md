# Changelog

## 0.2.1 – 2026-11-01

Security maintenance release. Updates russh to 0.65.0 to fix SSH client memory
exhaustion via EXT_INFO packets and stalled channel writes caused by a peer
advertising a zero maximum packet size. Also includes upstream agent-server
security fixes and updates the transitive pageant dependency to 0.2.5.

The affected agent-server and Pageant paths are not used by YuuTerm production
authentication, which connects to the Windows OpenSSH agent. The glib advisory
remains in the cross-platform lockfile; glib is not part of the Windows build.

Also updates crypto-bigint to 0.7.10, ctutils to 0.4.7 and rustls-webpki to
0.103.16 for timing and certificate-verification security fixes, and replaces
the yanked clipboard-win version with 5.4.3. The existing RSA timing advisory
and its documented limitation remain; no new advisory suppressions were added.

This preview remains unsigned.

## 0.2.0 – 2026-10-31

First public preview of YuuTerm, a terminal and remote session manager for
Windows 10 and 11 (x64): SSH with a file browser, RDP, VNC, Telnet, serial ports
and X11 forwarding. So far it has only been tested on Windows 11. Most of it was
vibe-coded, so expect bugs.

Pick the installer or the portable ZIP. The files aren't signed yet, so
SmartScreen will probably warn you about an unknown publisher. Known limitations
are in SECURITY.md, bundled licenses in THIRD_PARTY_NOTICES.md.

The X11 source ZIP is the source of the bundled Cygwin/X runtime. GitHub adds
the YuuTerm source code to this release automatically.
