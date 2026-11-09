# Pulse

**Write components in TypeScript + JSX. Ship WebAssembly.**

Pulse is a frontend framework with a familiar React-like authoring experience
(`.tsx` files, JSX, typed components) whose compiler, written in Rust, turns
your application into a WebAssembly module that drives the DOM directly.
Reactivity is built in: no virtual DOM, no diffing, no state-management library.
When a signal changes, Pulse updates exactly the text nodes and attributes
that read it.

```tsx
import { signal } from "pulse";

const count = signal<number>(0);

export default function App(): JSX.Element {
  return (
    <button onClick={() => count.value++}>
      Count: {count.value}
    </button>
  );
}
```

```bash
pulse build src/App.tsx --out dist
```

That's it: `dist/` holds `index.html`, `app.wasm` and a small loader. Serve it
with any static file server.

Pulse is **frontend-only and backend-agnostic**. It doesn't care what your
server is written in; your app talks to backends through standard web APIs.

> **Status:** early development. The [Status](#status) section below lists
> exactly what works today and what is planned. Anything marked *planned* in
> this README doesn't exist yet.

---

## Why Pulse?

| | |
|---|---|
| **Familiar** | TSX syntax, function components, `onClick={...}`. If you know React or Solid, you already know how to read Pulse code. |
| **Type-safe** | Pulse apps are TypeScript. The compiler type-checks your code (`number`, `string`, `boolean`, `Signal<T>`, ...) and maps it to real Rust types. Type errors are compile errors, with the exact line and column. |
| **Fast by construction** | Components run once. The compiler knows which parts of the UI are static and which depend on signals, so static markup is created once and only the dynamic parts get fine-grained effects. |
| **Small surface** | Five primitives: `signal`, `computed`, `effect`, `batch`, `untrack`. |
| **No backend lock-in** | It's just a static bundle. Host it anywhere. |

---

## Getting started

### 1. Install

You need Rust with the WebAssembly target:

```bash
rustup target add wasm32-unknown-unknown
```

Then install the `pulse` CLI from this repository:

```bash
cargo install --path crates/pulse-compiler
```

*(Planned: a published release, so this becomes `cargo install pulse-compiler`.)*

### 2. Create an app

A Pulse app is a folder with a TSX entry file:

```text
my-app/
├── src/
│   └── App.tsx        # your root component (export default)
├── index.html         # optional; a default page is generated if missing
└── tsconfig.json      # optional; for editor type-checking (see below)
```

`src/App.tsx`:

```tsx
import { signal, computed } from "pulse";

const count = signal<number>(0);
const doubled = computed(() => count.value * 2);

export default function App(): JSX.Element {
  return (
    <main class="app">
      <h1>Hello, Pulse</h1>
      <button onClick={() => count.value++}>Clicked {count.value} times</button>
      <p>Doubled: {doubled.value}</p>
    </main>
  );
}
```

*(Planned: `pulse new my-app` to scaffold this.)*

### 3. Build and run

```bash
pulse build src/App.tsx --out dist
```

```bash
python3 -m http.server 8080 --directory dist
```

Open <http://localhost:8080>. Any static server works; the browser just needs
`.wasm` files served as `application/wasm`.

*(Planned: `pulse dev` with file watching and a built-in dev server.)*

### Editor support

Pulse ships TypeScript declarations in [`types/pulse.d.ts`](types/pulse.d.ts),
so VS Code (or `tsc --noEmit`) understands `import { signal } from "pulse"` and
JSX. Point your `tsconfig.json` at them:

```json
{
  "compilerOptions": {
    "strict": true,
    "jsx": "preserve",
    "noEmit": true,
    "paths": { "pulse": ["../../types/pulse.d.ts"] }
  },
  "include": ["src", "../../types"]
}
```

Code that the Pulse compiler accepts also type-checks with `tsc`.

---

## The reactive model

### `signal`: reactive state

```tsx
const name = signal<string>("world");   // explicit type
const count = signal(0);                // inferred: Signal<number>

count.value;        // read (inside JSX/computed/effect this subscribes)
count.value = 5;    // write
count.value++;      // update
name.value += "!";  // string append
```

Signals declared at module level are **global shared state**; any component
can use them. Signals declared inside a component are **local state**:

```tsx
function Toggle(): JSX.Element {
  const on = signal(false);               // local to this component
  return <button onClick={() => on.value = !on.value}>{on.value ? "ON" : "OFF"}</button>;
}
```

### `computed`: derived values

```tsx
const price = signal(20);
const qty = signal(3);
const total = computed(() => price.value * qty.value);   // Computed<number>
```

Computed values are lazy and cached. They recalculate only when read after a
dependency changed. They are read-only: assigning `total.value = 1` is a compile
error.

### `effect`: side effects

```tsx
effect(() => {
  console.log("count is now " + count.value);
});
```

Runs immediately, then again whenever anything it read changes. Dependencies
are tracked automatically, including conditional ones.

### `batch`: grouped updates

```tsx
batch(() => {
  price.value = 25;
  qty.value = 4;
});   // effects run once here, not twice
```

Event handlers are batched automatically.

### `untrack`: read without subscribing

```tsx
effect(() => {
  console.log(count.value + " (step is " + untrack(() => step.value) + ")");
});
```

---

## Components and JSX

* A component is a function returning `JSX.Element`. The root component is
  the module's `export default function` (or a function named `App`).
* Use components as tags: `<Header />`.
* Attributes: `class="..."` (or `className`), dynamic `title={expr}`, boolean
  `disabled={isBusy.value}`.
* Events: `onClick`, `onInput`, `onMouseEnter`, ... take a function:
  `onClick={() => ...}` or `onClick={handler}`.
* Text: `{expr}` renders numbers and strings; adjacent text is merged into one
  DOM text node.
* Fragments `<>...</>` are allowed inside elements.

Plain helper functions work too, with typed parameters:

```tsx
function label(n: number): string {
  if (n === 1) {
    return "1 item";
  }
  return n + " items";
}
```

---

## How it works

```text
App.tsx ──► Lexer/Parser ──► AST ──► Type checker ──► Typed IR ──► Optimizer ──► Rust codegen ──► cargo (wasm32) ──► app.wasm
                                                                                                                       │
                                                         Browser:  index.html ─► pulse.js loader ─► instantiate ◄──────┘
                                                                                     ▲      │
                                                                         DOM calls   │      ▼  events
                                                                                  pulse-runtime (signals + DOM bindings)
```

* **`crates/pulse-compiler`**: the compiler and `pulse` CLI (Rust, no external
  dependencies). It parses TSX, checks TypeScript types, lowers JSX to a DOM
  tree IR, optimizes it (constant folding, text merging, static vs. reactive
  analysis), and emits Rust that is compiled to `wasm32-unknown-unknown`.
* **`crates/pulse-runtime`**: linked into every app. The reactive graph
  (push–pull signals, lazy computed values, batched effects) plus direct DOM
  bindings through a tiny import interface (`create_element`, `set_text`,
  `append_child`, `add_event_listener`, ...). It has no `wasm-bindgen` and no
  dependencies.
* **`runtime/pulse.js`**: a ~100-line loader that instantiates the module and
  gives it access to the DOM.
* **`types/pulse.d.ts`**: TypeScript declarations for editors.

Generating Rust (rather than raw WASM bytes) gives Pulse the full Rust
optimizer and type system as a safety net for free. The generated code is
readable. Run `pulse emit src/App.tsx` to see it.

---

## CLI reference

| Command | What it does |
|---|---|
| `pulse build <entry.tsx> [--out dist]` | Compile the app to `dist/` (`index.html`, `app.wasm`, `pulse.js`). |
| `pulse check <entry.tsx>` | Parse and type-check only; fast feedback. |
| `pulse emit <entry.tsx>` | Print the generated Rust code. |
| `pulse new <name>` | *Planned*: scaffold a new app. |
| `pulse dev <entry.tsx>` | *Planned*: watch, rebuild and serve. |

---

## Status

Milestone 1: a minimal end-to-end prototype.

- [x] Reactive runtime: `signal`, `computed`, `effect`, `batch`, `untrack`, automatic dependency tracking (unit-tested)
- [x] DOM bindings through WASM imports, plus an in-memory DOM for native tests
- [x] TSX lexer and parser (components, JSX, arrows, operators, `if`/`return`, generic calls)
- [x] TypeScript type checking to a typed IR
- [x] Optimizer: constant folding, text-node merging
- [ ] Rust code generation
- [ ] `pulse build` / `check` / `emit` CLI
- [ ] JS loader and default `index.html`
- [ ] Counter example running in a real browser (end-to-end test)
- [ ] `types/pulse.d.ts`

The [roadmap](ROADMAP.md) covers what comes next: component props, reactive
conditionals and lists, input events, cleanup/ownership, a dev server and
more.

## Not yet supported

So that nothing surprises you: props, `if`/ternary rendering that switches
elements reactively, lists (`.map`), event objects (`e.target.value`), arrays,
objects, interfaces, template literals, loops, `async`, and `fetch`. Using any
of them gives a clear compile error rather than wrong output.

## Development

```bash
cargo test --workspace
```

## License

MIT
