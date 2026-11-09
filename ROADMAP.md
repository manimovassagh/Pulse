# Pulse roadmap

Each milestone ends with something that runs in a browser and is covered by tests.

## Milestone 1: end-to-end prototype (in progress)
TSX → parser → typed IR → optimizer → Rust → WASM → browser.
Counter example: render, click, and reactive update via signals, computed and effects.

## Milestone 2: real components
- Component props (`function Card(props: { title: string })`), children
- Reactive conditional rendering (`{cond ? <A/> : <B/>}`, `{cond && <A/>}`)
- Lists with keyed reconciliation (`items.value.map(...)`), arrays in signals
- Event objects: `onInput={(e) => name.value = e.target.value}`
- Ownership and cleanup: dispose effects and listeners when a subtree is removed

## Milestone 3: developer experience
- `pulse new`, and `pulse dev` (watch + rebuild + static server)
- Multiple modules / `import` between app files
- Template literals, `interface` / `type` aliases, object types
- Faster rebuilds (cached runtime build, incremental codegen)

## Milestone 4: talking to backends
- `fetch` bindings with async/await (or a resource primitive)
- `localStorage`, timers (`setTimeout`, `setInterval`)

## Later
- Equality cut-off for computed values; memory reclamation in the reactive graph
- Direct WASM emission for faster builds (keeping Rust codegen as reference)
- Source maps from WASM back to TSX
- SSR-free static pre-rendering
