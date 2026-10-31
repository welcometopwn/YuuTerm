# Third-party notices

The MIT license covers YuuTerm's own code. Dependencies retain their own licenses.
Release packages include a `third-party` directory with dependency inventories and
license texts, and the bundled X server's `x11/licenses` and `x11/SOURCES.txt`.

The bundled Cygwin/X runtime is distributed unmodified. Cygwin's API library uses
LGPL-3.0-or-later; other runtime packages use their respective licenses, including
GPL licenses. Matching source archives are distributed alongside release binaries
in `YuuTerm-0.2.0-x11-sources.zip`. Preserve these sources and notices when redistributing.

The bundled JetBrains Mono Nerd Font uses the SIL Open Font License; its license
text is included in the dependency bundle and in `ui/assets/fonts/OFL.txt`.

The app-local Microsoft Visual C++ Runtime remains subject to Microsoft's runtime
redistribution terms. WebView2 and the Windows Remote Desktop control are Microsoft
components. The installer downloads WebView2 only if it is missing; it is not
relicensed under YuuTerm's MIT license.
