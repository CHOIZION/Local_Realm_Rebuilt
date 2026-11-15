# Recovered frontend distribution

These are the 23 original static assets recovered from the supplied Local Realm 1.9.0 Windows executable. They preserve the existing Svelte UI and map/editor code. They are not the original `.svelte` or TypeScript development sources.

Do not edit the minified bundles to maintain new behavior. Readable adapters live in `frontend/`, and `tools/build-frontend.mjs` applies exact, checked transformations into `dist/frontend/`. A mismatched recovery input fails the build instead of silently applying a partial transformation.

Original executable SHA-256: `0779aa707878cfdb4b1c2b5137e71769a06a35a5f248074251421d3ec8bb3112`.
