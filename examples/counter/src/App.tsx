import { signal, computed, effect, batch } from "pulse";
import "./styles.css";

// Global shared state: any component can read or update it.
const count = signal<number>(0);
const doubled = computed(() => count.value * 2);

function label(n: number): string {
  if (n === 1) {
    return "1 click";
  }
  return n + " clicks";
}

function Stats(): JSX.Element {
  return (
    <p class="stats">
      Doubled: {doubled.value} · Parity: {count.value % 2 === 0 ? "even" : "odd"}
    </p>
  );
}

function Toggle(): JSX.Element {
  // Local component state.
  const on = signal(false);
  return (
    <button class="toggle" onClick={() => (on.value = !on.value)} aria-pressed={on.value ? "true" : "false"}>
      Light is {on.value ? "ON" : "OFF"}
    </button>
  );
}

export default function App(): JSX.Element {
  const step = signal(1);
  const reset = (): void => {
    batch(() => {
      count.value = 0;
      step.value = 1;
    });
  };

  effect(() => {
    console.log("count changed to " + count.value);
  });

  return (
    <main class="app">
      <h1>Pulse Counter</h1>
      <button id="increment" onClick={() => (count.value += step.value)}>
        Count: {count.value}
      </button>
      <button id="step" onClick={() => step.value++}>
        Step: {step.value}
      </button>
      <button id="reset" onClick={reset} disabled={count.value === 0}>
        Reset
      </button>
      <p id="label">You clicked {label(count.value)}.</p>
      <Stats />
      <Toggle />
    </main>
  );
}
