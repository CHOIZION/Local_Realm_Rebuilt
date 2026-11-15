# Third-party notices

## Rust dependencies

`src-tauri/Cargo.lock` records exact resolved versions. `licenses/rust/README.md` lists package license declarations and upstream repositories. Available license texts are copied beneath that directory by `tools/collect-licenses.ps1`.

## Recovered frontend

The HTML, JavaScript, CSS and images under `recovered-frontend/` were extracted from the supplied Local Realm 1.9.0 executable (SHA-256 `0779aa707878cfdb4b1c2b5137e71769a06a35a5f248074251421d3ec8bb3112`). They are compiled distribution assets, not reconstructed original Svelte sources. The original frontend package manifest, lockfile and complete dependency notices were not present in the supplied folder. They have not been invented here.

Identifiable frontend components include Svelte/SvelteKit, CodeMirror, Three.js, Cytoscape.js and D3. Their upstream terms continue to apply; this document does not relicense them. The exact original dependency inventory still needs reconciliation with the original project's dependency records if those records become available.

## Map brushes

The supplied brush packs are K. M. Alexander's cartography brushes: de Fer Cartography, Hyacinth, Mercator Settlement, Moronobu Mokuhanga, Qiánlóng and Van der Aa. The author's brush collection is distributed under CC0. See the [author's brush page](https://kmalexander.com/tag/brushes/), [de Fer release](https://kmalexander.com/2022/06/28/de-fer-cartography-a-free-17th-century-brush-set-for-fantasy-maps/) and [Qiánlóng release](https://kmalexander.com/2026/02/02/qianlong-a-free-18th-century-cartography-brush-set-for-fantasy-maps/).

## Fallback print font

Nanum Gothic, copyright NHN Corporation, is distributed under the SIL Open Font License 1.1. The unmodified font and its license are in `src-tauri/resources/fonts/`; a copy of the license accompanies releases. Source: [Google Fonts' Nanum Gothic directory](https://github.com/google/fonts/tree/main/ofl/nanumgothic).

## Microsoft WebView2

Portable packages may include the fixed WebView2 runtime supplied with the original application. Microsoft WebView2 remains subject to Microsoft's license and the notices accompanying that runtime. It is not part of this project's own source license. The runtime is excluded from the source archive.

## Project license

No license has been selected for the owner's original assets or the reconstructed application code. The owner can choose a project license independently of the third-party licenses above.
