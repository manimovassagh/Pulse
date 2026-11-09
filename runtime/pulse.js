// Pulse browser loader.
//
// Instantiates a Pulse application's WebAssembly module and gives it access
// to the DOM. The module refers to DOM nodes by integer handles (indices into
// `nodes`) and passes strings as (pointer, length) pairs of UTF-8 in its
// linear memory. This file has no dependencies and needs no build step.

/** The running module, so a later `start` can hot-swap it. */
let current = null;

/**
 * Loads and starts a Pulse app.
 *
 * Calling it again replaces the running app with a new module (hot reload).
 * If both modules were built by `pulse dev`, global signal values are carried
 * over to the new module.
 *
 * @param {string | URL} wasmUrl URL of the compiled `app.wasm`.
 * @returns {Promise<WebAssembly.Instance>}
 */
export async function start(wasmUrl) {
  const snapshot = current ? saveState(current.exports) : null;

  /** @type {(Node | null)[]} handle → DOM node; 0 is reserved as "null". */
  const nodes = [null];
  const decoder = new TextDecoder();
  let memory = null;
  let exports = null;

  const str = (ptr, len) => decoder.decode(new Uint8Array(memory.buffer, ptr, len));
  const add = (node) => nodes.push(node) - 1;

  const imports = {
    pulse: {
      create_element: (ptr, len) => add(document.createElement(str(ptr, len))),
      create_text: (ptr, len) => add(document.createTextNode(str(ptr, len))),
      set_text: (node, ptr, len) => {
        nodes[node].data = str(ptr, len);
      },
      set_attribute: (node, np, nl, vp, vl) => nodes[node].setAttribute(str(np, nl), str(vp, vl)),
      remove_attribute: (node, ptr, len) => nodes[node].removeAttribute(str(ptr, len)),
      append_child: (parent, child) => {
        nodes[parent].appendChild(nodes[child]);
      },
      add_event_listener: (node, ptr, len, handler) => {
        nodes[node].addEventListener(str(ptr, len), () => exports.pulse_dispatch(handler));
      },
      mount: (node, ptr, len) => {
        const target = document.querySelector(str(ptr, len));
        if (!target) return 0;
        target.replaceChildren(nodes[node]);
        return 1;
      },
      console_log: (ptr, len) => console.log(str(ptr, len)),
      console_error: (ptr, len) => console.error(str(ptr, len)),
    },
  };

  const response = fetch(wasmUrl);
  let instance;
  try {
    ({ instance } = await WebAssembly.instantiateStreaming(response, imports));
  } catch (err) {
    // Servers that do not send `Content-Type: application/wasm` break
    // streaming compilation; fall back to compiling from bytes.
    const bytes = await (await fetch(wasmUrl)).arrayBuffer();
    ({ instance } = await WebAssembly.instantiate(bytes, imports));
  }
  memory = instance.exports.memory;
  exports = instance.exports;
  if (snapshot && exports.pulse_hot_restore) {
    const ptr = exports.pulse_hot_alloc(snapshot.length);
    new Uint8Array(memory.buffer, ptr, snapshot.length).set(snapshot);
    exports.pulse_hot_restore();
  }
  exports.pulse_start();
  current = instance;
  return instance;
}

/** Copies the old module's state snapshot out of its memory. */
function saveState(exports) {
  if (!exports.pulse_hot_save) return null;
  const ptr = exports.pulse_hot_save();
  const len = exports.pulse_hot_len();
  return new Uint8Array(exports.memory.buffer, ptr, len).slice();
}
