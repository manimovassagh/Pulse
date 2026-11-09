//! State transfer for hot reloading during development.
//!
//! When `pulse dev` swaps in a freshly compiled module, the old module saves
//! the values of its global signals into a snapshot, the JavaScript loader
//! copies the bytes into the new module's memory, and the new module restores
//! every value whose name and type still match. Only development builds
//! export these hooks.
//!
//! Snapshot format: one `name=tag:value` line per signal, where tag is `n`
//! (number), `s` (string, with `\n` and `\\` escaped) or `b` (boolean).

use std::cell::RefCell;

thread_local! {
    /// Bytes of the last snapshot, or of a snapshot being restored.
    static BUFFER: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// A value that can survive a hot reload.
pub trait HotValue: Sized {
    fn encode(&self) -> String;
    fn decode(s: &str) -> Option<Self>;
}

impl HotValue for f64 {
    fn encode(&self) -> String {
        format!("n:{self}")
    }
    fn decode(s: &str) -> Option<Self> {
        s.strip_prefix("n:")?.parse().ok()
    }
}

impl HotValue for bool {
    fn encode(&self) -> String {
        format!("b:{}", u8::from(*self))
    }
    fn decode(s: &str) -> Option<Self> {
        match s.strip_prefix("b:")? {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        }
    }
}

impl HotValue for String {
    fn encode(&self) -> String {
        format!("s:{}", self.replace('\\', "\\\\").replace('\n', "\\n"))
    }
    fn decode(s: &str) -> Option<Self> {
        let raw = s.strip_prefix("s:")?;
        let mut out = String::with_capacity(raw.len());
        let mut chars = raw.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next()? {
                    'n' => out.push('\n'),
                    other => out.push(other),
                }
            } else {
                out.push(c);
            }
        }
        Some(out)
    }
}

/// Builds a snapshot of named values.
#[derive(Default)]
pub struct Snapshot {
    text: String,
}

impl Snapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put<T: HotValue>(&mut self, name: &str, value: &T) {
        self.text.push_str(name);
        self.text.push('=');
        self.text.push_str(&value.encode());
        self.text.push('\n');
    }

    /// Stores the snapshot in the shared buffer and returns a pointer to it;
    /// the length is available from [`len`].
    pub fn finish(self) -> *const u8 {
        BUFFER.with(|b| {
            let mut b = b.borrow_mut();
            *b = self.text.into_bytes();
            b.as_ptr()
        })
    }
}

/// Length in bytes of the buffer.
pub fn len() -> usize {
    BUFFER.with(|b| b.borrow().len())
}

/// Reserves `len` bytes in the buffer for the host to write a snapshot into.
pub fn alloc(len: usize) -> *mut u8 {
    BUFFER.with(|b| {
        let mut b = b.borrow_mut();
        *b = vec![0; len];
        b.as_mut_ptr()
    })
}

/// A snapshot received from the previous module.
pub struct Restored {
    text: String,
}

/// Takes the snapshot the host wrote with [`alloc`].
pub fn take_restored() -> Restored {
    let bytes = BUFFER.with(|b| std::mem::take(&mut *b.borrow_mut()));
    Restored { text: String::from_utf8(bytes).unwrap_or_default() }
}

impl Restored {
    /// The value saved under `name`, if it exists and has type `T`.
    pub fn get<T: HotValue>(&self, name: &str) -> Option<T> {
        let line = self.text.lines().find(|l| l.split_once('=').is_some_and(|(n, _)| n == name))?;
        T::decode(line.split_once('=')?.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut s = Snapshot::new();
        s.put("count", &42.5);
        s.put("name", &String::from("a\\b\nc=d"));
        s.put("on", &true);
        let ptr = s.finish();
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len()) }.to_vec();

        let dst = alloc(bytes.len());
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, bytes.len()) };
        let r = take_restored();
        assert_eq!(r.get::<f64>("count"), Some(42.5));
        assert_eq!(r.get::<String>("name").as_deref(), Some("a\\b\nc=d"));
        assert_eq!(r.get::<bool>("on"), Some(true));
        // Type changed between reloads: the value is dropped, not misread.
        assert_eq!(r.get::<String>("count"), None);
        assert_eq!(r.get::<f64>("missing"), None);
    }
}
