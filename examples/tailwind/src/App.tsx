import { signal, computed } from "pulse";
import "./styles.css";

// Tailwind classes are plain strings, so the Tailwind CLI finds them in this
// file, including the ones that are switched reactively below.
const dark = signal(false);
const likes = signal(0);
const liked = computed(() => likes.value > 0);

function ThemeToggle(): JSX.Element {
  return (
    <button
      id="theme"
      class="rounded-full border border-slate-300 px-3 py-1 text-sm hover:bg-slate-100 dark:border-slate-600 dark:hover:bg-slate-800"
      onClick={() => (dark.value = !dark.value)}
    >
      {dark.value ? "☀️ Light" : "🌙 Dark"}
    </button>
  );
}

export default function App(): JSX.Element {
  return (
    <div class={dark.value ? "dark" : ""}>
      <main class="flex min-h-screen items-center justify-center bg-slate-50 p-6 transition-colors dark:bg-slate-950">
        <section
          id="card"
          class="w-full max-w-sm rounded-2xl bg-white p-8 text-center shadow-xl ring-1 ring-slate-900/5 dark:bg-slate-900 dark:text-slate-100"
        >
          <img src="logo.svg" alt="Pulse logo" class="mx-auto mb-4 h-16 w-16" />
          <h1 class="text-2xl font-bold tracking-tight">Pulse + Tailwind</h1>
          <p class="mt-2 text-slate-500 dark:text-slate-400">
            Utility classes compiled by Tailwind, components compiled to WebAssembly.
          </p>
          <div class="mt-6 flex items-center justify-center gap-3">
            <button
              id="like"
              class={
                liked.value
                  ? "rounded-lg bg-pink-600 px-4 py-2 font-semibold text-white shadow hover:bg-pink-500"
                  : "rounded-lg bg-slate-900 px-4 py-2 font-semibold text-white shadow hover:bg-slate-700 dark:bg-slate-700"
              }
              onClick={() => likes.value++}
            >
              {liked.value ? "♥ " + likes.value : "♡ Like"}
            </button>
            <ThemeToggle />
          </div>
        </section>
      </main>
    </div>
  );
}
