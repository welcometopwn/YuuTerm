# Bundled terminal font

JetBrains Mono NL Nerd Font **Mono**, Nerd Fonts **v3.5.1**, from
[the upstream release tree](https://github.com/ryanoasis/nerd-fonts/tree/v3.5.1/patched-fonts/JetBrainsMono/NoLigatures).
Distributed under the adjacent unmodified `OFL.txt` (SIL Open Font License 1.1).

Regular, Bold, Italic and BoldItalic are bundled locally as WOFF2. No Windows font
installation or font CDN is used. The CSS family alias is `YuuTerm JetBrains Mono`.
The NL variant avoids ligatures; Mono keeps prompt icons in terminal cells.

Original TTF SHA-256 values:

| Face | SHA-256 |
| --- | --- |
| Regular | d9a80146bbf2ff23b187316a3e82ee2a2ceab8220267fe09ac150ca46ab2c3bd |
| Bold | f799fc4b215c7b7254bf6cee51c0bce122ac684b53b46fd3cb83d408a0ccf385 |
| Italic | 336a3cef8770dab01c0fc9c6bb0c0473bfa82f90a4033fde2cc52d8aa410528c |
| BoldItalic | 28ce03bcaf808f7fa64dcce6facb00319083bdb04cc4f61d41b9f6ad8f20c3e8 |

Converted without subsetting or glyph edits with fonttools 4.60.1 / brotli 1.2.0:
`font = TTFont(source); font.flavor = "woff2"; font.save(destination)`.
Every converted face's cmap was compared with its TTF source; U+E0B0/U+E0B2
Powerline separators, U+F07B folder, U+F120 terminal and U+F126 branch are present.
Four faces add approximately 4.1 MiB to the frontend assets.
