//! Direct DOM bindings.
//!
//! On `wasm32` every operation is a call into a handful of functions that the
//! JavaScript loader (`runtime/pulse.js`) provides under the `pulse` import
//! module. DOM nodes live on the JavaScript side and are referred to by a
//! numeric handle ([`Node`]). Strings cross the boundary as `(ptr, len)` pairs
//! pointing into WASM memory, so no serialization library is needed.
//!
//! On every other target the same API is backed by a small in-memory DOM
//! ([`mock`]), which lets compiled components be rendered and clicked in
//! ordinary `cargo test` runs.

use std::cell::RefCell;
use std::rc::Rc;

/// Handle to a DOM node owned by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Node(pub u32);

type Handler = Rc<RefCell<dyn FnMut()>>;

thread_local! {
    static HANDLERS: RefCell<Vec<Handler>> = const { RefCell::new(Vec::new()) };
}

/// Creates an element such as `div` or `button`.
pub fn create_element(tag: &str) -> Node {
    Node(host::create_element(tag))
}

/// Creates a text node.
pub fn create_text(text: &str) -> Node {
    Node(host::create_text(text))
}

/// Replaces the content of a text node.
pub fn set_text(node: Node, text: &str) {
    host::set_text(node.0, text)
}

/// Sets an attribute (`class`, `id`, `href`, ...).
pub fn set_attribute(node: Node, name: &str, value: &str) {
    host::set_attribute(node.0, name, value)
}

/// Adds (`true`) or removes (`false`) a boolean attribute such as `disabled`.
pub fn set_bool_attribute(node: Node, name: &str, present: bool) {
    if present {
        host::set_attribute(node.0, name, "")
    } else {
        host::remove_attribute(node.0, name)
    }
}

/// Appends `child` as the last child of `parent`.
pub fn append_child(parent: Node, child: Node) {
    host::append_child(parent.0, child.0)
}

/// Attaches an event listener. All writes performed by the handler are
/// batched, so dependent effects run once after the handler returns.
pub fn on(node: Node, event: &str, handler: impl FnMut() + 'static) {
    let id = HANDLERS.with(|h| {
        let mut h = h.borrow_mut();
        h.push(Rc::new(RefCell::new(handler)));
        (h.len() - 1) as u32
    });
    host::add_event_listener(node.0, event, id)
}

/// Invokes the event handler registered under `id`. Called by the host.
pub fn dispatch(id: u32) {
    let handler = HANDLERS.with(|h| h.borrow().get(id as usize).cloned());
    if let Some(handler) = handler {
        crate::reactive::batch(|| (handler.borrow_mut())());
    }
}

/// Replaces the children of the element matching `selector` with `node`.
///
/// # Panics
/// If no element matches the selector.
pub fn mount(node: Node, selector: &str) {
    if !host::mount(node.0, selector) {
        panic!("pulse: mount target `{selector}` not found");
    }
}

/// Writes a line to the browser console (`console.log` in TSX).
pub fn console_log(message: &str) {
    host::console_log(message)
}

/// Routes Rust panics to `console.error` so failures are visible in the
/// browser. Called once by the generated entry point.
pub fn install_panic_hook() {
    #[cfg(target_arch = "wasm32")]
    std::panic::set_hook(Box::new(|info| host::console_error(&info.to_string())));
}

#[cfg(target_arch = "wasm32")]
mod host {
    #[link(wasm_import_module = "pulse")]
    extern "C" {
        #[link_name = "create_element"]
        fn js_create_element(ptr: *const u8, len: usize) -> u32;
        #[link_name = "create_text"]
        fn js_create_text(ptr: *const u8, len: usize) -> u32;
        #[link_name = "set_text"]
        fn js_set_text(node: u32, ptr: *const u8, len: usize);
        #[link_name = "set_attribute"]
        fn js_set_attribute(node: u32, np: *const u8, nl: usize, vp: *const u8, vl: usize);
        #[link_name = "remove_attribute"]
        fn js_remove_attribute(node: u32, ptr: *const u8, len: usize);
        #[link_name = "append_child"]
        fn js_append_child(parent: u32, child: u32);
        #[link_name = "add_event_listener"]
        fn js_add_event_listener(node: u32, ptr: *const u8, len: usize, handler: u32);
        #[link_name = "mount"]
        fn js_mount(node: u32, ptr: *const u8, len: usize) -> u32;
        #[link_name = "console_log"]
        fn js_console_log(ptr: *const u8, len: usize);
        #[link_name = "console_error"]
        fn js_console_error(ptr: *const u8, len: usize);
    }

    // SAFETY (all functions below): the host only reads `len` bytes of UTF-8
    // starting at `ptr` during the call, and node handles are opaque integers
    // the host validates itself.
    pub fn create_element(tag: &str) -> u32 {
        unsafe { js_create_element(tag.as_ptr(), tag.len()) }
    }
    pub fn create_text(text: &str) -> u32 {
        unsafe { js_create_text(text.as_ptr(), text.len()) }
    }
    pub fn set_text(node: u32, text: &str) {
        unsafe { js_set_text(node, text.as_ptr(), text.len()) }
    }
    pub fn set_attribute(node: u32, name: &str, value: &str) {
        unsafe { js_set_attribute(node, name.as_ptr(), name.len(), value.as_ptr(), value.len()) }
    }
    pub fn remove_attribute(node: u32, name: &str) {
        unsafe { js_remove_attribute(node, name.as_ptr(), name.len()) }
    }
    pub fn append_child(parent: u32, child: u32) {
        unsafe { js_append_child(parent, child) }
    }
    pub fn add_event_listener(node: u32, event: &str, handler: u32) {
        unsafe { js_add_event_listener(node, event.as_ptr(), event.len(), handler) }
    }
    pub fn mount(node: u32, selector: &str) -> bool {
        unsafe { js_mount(node, selector.as_ptr(), selector.len()) != 0 }
    }
    pub fn console_log(msg: &str) {
        unsafe { js_console_log(msg.as_ptr(), msg.len()) }
    }
    pub fn console_error(msg: &str) {
        unsafe { js_console_error(msg.as_ptr(), msg.len()) }
    }
}

#[cfg(not(target_arch = "wasm32"))]
use mock::host;

/// In-memory DOM used when not compiling to WebAssembly (tests, tooling).
#[cfg(not(target_arch = "wasm32"))]
pub mod mock {
    use super::Node;
    use std::cell::RefCell;

    enum Kind {
        Element { tag: String, attrs: Vec<(String, String)> },
        Text(String),
    }

    struct MockNode {
        kind: Kind,
        children: Vec<u32>,
        listeners: Vec<(String, u32)>,
    }

    #[derive(Default)]
    struct Document {
        nodes: Vec<MockNode>,
        /// `(selector, mounted node)` pairs.
        roots: Vec<(String, u32)>,
        log: Vec<String>,
    }

    thread_local! {
        static DOC: RefCell<Document> = RefCell::new(Document::default());
    }

    fn with_doc<R>(f: impl FnOnce(&mut Document) -> R) -> R {
        DOC.with(|d| f(&mut d.borrow_mut()))
    }

    fn add(kind: Kind) -> u32 {
        with_doc(|d| {
            d.nodes.push(MockNode { kind, children: Vec::new(), listeners: Vec::new() });
            (d.nodes.len() - 1) as u32
        })
    }

    pub(super) mod host {
        use super::*;

        pub fn create_element(tag: &str) -> u32 {
            add(Kind::Element { tag: tag.into(), attrs: Vec::new() })
        }
        pub fn create_text(text: &str) -> u32 {
            add(Kind::Text(text.into()))
        }
        pub fn set_text(node: u32, text: &str) {
            with_doc(|d| {
                if let Kind::Text(t) = &mut d.nodes[node as usize].kind {
                    *t = text.into();
                }
            })
        }
        pub fn set_attribute(node: u32, name: &str, value: &str) {
            with_doc(|d| {
                if let Kind::Element { attrs, .. } = &mut d.nodes[node as usize].kind {
                    match attrs.iter_mut().find(|(n, _)| n == name) {
                        Some(slot) => slot.1 = value.into(),
                        None => attrs.push((name.into(), value.into())),
                    }
                }
            })
        }
        pub fn remove_attribute(node: u32, name: &str) {
            with_doc(|d| {
                if let Kind::Element { attrs, .. } = &mut d.nodes[node as usize].kind {
                    attrs.retain(|(n, _)| n != name);
                }
            })
        }
        pub fn append_child(parent: u32, child: u32) {
            with_doc(|d| d.nodes[parent as usize].children.push(child))
        }
        pub fn add_event_listener(node: u32, event: &str, handler: u32) {
            with_doc(|d| d.nodes[node as usize].listeners.push((event.into(), handler)))
        }
        pub fn mount(node: u32, selector: &str) -> bool {
            with_doc(|d| {
                d.roots.retain(|(s, _)| s != selector);
                d.roots.push((selector.into(), node));
            });
            true
        }
        pub fn console_log(msg: &str) {
            with_doc(|d| d.log.push(msg.into()))
        }
    }

    fn escape(s: &str) -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
    }

    fn write_html(d: &Document, id: u32, out: &mut String) {
        let node = &d.nodes[id as usize];
        match &node.kind {
            Kind::Text(t) => out.push_str(&escape(t)),
            Kind::Element { tag, attrs } => {
                out.push('<');
                out.push_str(tag);
                for (n, v) in attrs {
                    out.push_str(&format!(" {n}=\"{}\"", escape(v)));
                }
                out.push('>');
                for &c in &node.children {
                    write_html(d, c, out);
                }
                out.push_str(&format!("</{tag}>"));
            }
        }
    }

    /// Serializes a node and its subtree to HTML.
    pub fn html(node: Node) -> String {
        let mut out = String::new();
        with_doc(|d| write_html(d, node.0, &mut out));
        out
    }

    /// HTML of whatever is mounted at `selector`.
    pub fn mounted_html(selector: &str) -> Option<String> {
        let root = with_doc(|d| d.roots.iter().find(|(s, _)| s == selector).map(|r| r.1))?;
        Some(html(Node(root)))
    }

    /// Finds elements with the given tag in document order below `root`.
    pub fn find_all(root: Node, tag: &str) -> Vec<Node> {
        fn walk(d: &Document, id: u32, tag: &str, out: &mut Vec<Node>) {
            let n = &d.nodes[id as usize];
            if matches!(&n.kind, Kind::Element { tag: t, .. } if t == tag) {
                out.push(Node(id));
            }
            for &c in &n.children {
                walk(d, c, tag, out);
            }
        }
        let mut out = Vec::new();
        with_doc(|d| walk(d, root.0, tag, &mut out));
        out
    }

    /// The node mounted at `selector`.
    pub fn root(selector: &str) -> Option<Node> {
        with_doc(|d| d.roots.iter().find(|(s, _)| s == selector).map(|r| Node(r.1)))
    }

    /// Fires `event` on `node`, invoking its listeners like a browser would.
    pub fn fire(node: Node, event: &str) {
        let ids: Vec<u32> = with_doc(|d| {
            d.nodes[node.0 as usize]
                .listeners
                .iter()
                .filter(|(e, _)| e == event)
                .map(|(_, id)| *id)
                .collect()
        });
        for id in ids {
            super::dispatch(id);
        }
    }

    /// Messages written with [`super::console_log`].
    pub fn console_messages() -> Vec<String> {
        with_doc(|d| d.log.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reactive::signal;
    use crate::text::ToText;

    #[test]
    fn renders_and_reacts_to_clicks() {
        let count = signal(0.0);
        let button = create_element("button");
        set_attribute(button, "class", "btn");
        let text = create_text("");
        crate::reactive::effect(move || set_text(text, &format!("Count: {}", count.get().to_text())));
        append_child(button, text);
        on(button, "click", move || count.update(|v| *v += 1.0));
        mount(button, "#app");

        assert_eq!(mock::mounted_html("#app").unwrap(), r#"<button class="btn">Count: 0</button>"#);
        mock::fire(button, "click");
        mock::fire(button, "click");
        assert_eq!(mock::html(button), r#"<button class="btn">Count: 2</button>"#);
    }

    #[test]
    fn bool_attributes() {
        let input = create_element("input");
        set_bool_attribute(input, "disabled", true);
        assert_eq!(mock::html(input), r#"<input disabled=""></input>"#);
        set_bool_attribute(input, "disabled", false);
        assert_eq!(mock::html(input), "<input></input>");
    }
}
