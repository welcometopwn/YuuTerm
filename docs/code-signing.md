# Code signing policy

YuuTerm 0.2.0 is not signed. I plan to apply to the SignPath Foundation for free
code signing for open-source projects; nothing has been approved yet.

## Team

welcometopwn writes, reviews and approves everything, including signing
requests. Contact: welcometopwn@outlook.com

## How releases get signed

- Only builds made by this repository's GitHub Actions workflow from the
  published source are submitted for signing.
- I approve every release by hand before it is signed.
- Bundled third-party programs and libraries (for example the X server) keep
  their own publisher and are not signed as YuuTerm.
- Signing credentials live only in protected CI secrets.

Until signing is active, every download and release note says the files are
unsigned.

## Privacy

YuuTerm doesn't send data anywhere unless you start a connection or transfer
yourself. Details are in [SECURITY.md](../SECURITY.md#privacy).
