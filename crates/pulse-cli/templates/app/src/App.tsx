import { signal } from "pulse";
import "./styles.css";

const count = signal(0);

export default function App(): JSX.Element {
  return (
    <main class="app">
      <img src="logo.svg" alt="Pulse" class="logo" />
      <h1>Hello, Pulse</h1>
      <button onClick={() => count.value++}>Count: {count.value}</button>
      <p class="hint">
        Edit <code>src/App.tsx</code> and save: the page updates instantly.
      </p>
    </main>
  );
}
