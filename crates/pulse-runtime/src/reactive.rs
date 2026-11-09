//! Fine-grained reactive core: signals, computed values, effects and batching.
//!
//! Every reactive primitive is a node in a single thread-local graph. Nodes are
//! addressed by index, which makes the public handles ([`Signal`],
//! [`Computed`]) small `Copy` values that closures can capture freely.
//!
//! The update algorithm is push–pull:
//!
//! * Writing a signal *pushes* a "stale" mark through the graph: computed nodes
//!   become dirty and effects are queued. Nothing is recomputed yet.
//! * Effects are then flushed (immediately, or when the outermost [`batch`]
//!   ends). Reading a dirty computed value *pulls* a recomputation on demand.
//!
//! Dependencies are tracked automatically: whatever a computed/effect reads
//! while it runs becomes its source set, which is rebuilt on every run so
//! conditional dependencies stay accurate.
//!
//! Limitations of this first version (tracked in ROADMAP.md): nodes are never
//! freed (there is no ownership/disposal yet), and a computed value that is
//! recomputed to an equal value still re-runs downstream effects.

use std::any::Any;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;

type NodeId = usize;

/// Upper bound on effect executions in a single flush; exceeding it almost
/// certainly means two effects keep re-triggering each other.
const MAX_FLUSH_ITERATIONS: usize = 100_000;

enum Kind {
    Signal,
    Computed(Rc<dyn Fn() -> Box<dyn Any>>),
    Effect(Rc<RefCell<dyn FnMut()>>),
}

struct Node {
    kind: Kind,
    /// Current value for signals and computed nodes (`None` while a value is
    /// temporarily taken out by [`Signal::update`], or before first compute).
    value: Option<Box<dyn Any>>,
    /// Computed only: value must be recomputed before it is read.
    dirty: bool,
    /// Effect only: already waiting in the queue.
    queued: bool,
    /// Nodes this node read during its last run.
    sources: Vec<NodeId>,
    /// Nodes that read this node during their last run.
    subscribers: Vec<NodeId>,
}

#[derive(Default)]
struct Runtime {
    nodes: Vec<Node>,
    observer: Option<NodeId>,
    batch_depth: u32,
    queue: VecDeque<NodeId>,
    flushing: bool,
}

thread_local! {
    static RT: RefCell<Runtime> = RefCell::new(Runtime::default());
}

fn with_rt<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    RT.with(|rt| f(&mut rt.borrow_mut()))
}

impl Runtime {
    fn add(&mut self, kind: Kind, value: Option<Box<dyn Any>>, dirty: bool) -> NodeId {
        self.nodes.push(Node {
            kind,
            value,
            dirty,
            queued: false,
            sources: Vec::new(),
            subscribers: Vec::new(),
        });
        self.nodes.len() - 1
    }

    /// Records that the currently running observer depends on `id`.
    fn track(&mut self, id: NodeId) {
        if let Some(obs) = self.observer {
            if obs != id && !self.nodes[obs].sources.contains(&id) {
                self.nodes[obs].sources.push(id);
                self.nodes[id].subscribers.push(obs);
            }
        }
    }

    /// Unlinks `id` from all of its sources before it re-runs.
    fn clear_sources(&mut self, id: NodeId) {
        for src in std::mem::take(&mut self.nodes[id].sources) {
            self.nodes[src].subscribers.retain(|&s| s != id);
        }
    }

    /// Propagates a change of `id` to everything downstream of it.
    fn mark_subscribers(&mut self, id: NodeId) {
        let mut stack = self.nodes[id].subscribers.clone();
        while let Some(sub) = stack.pop() {
            let node = &mut self.nodes[sub];
            match node.kind {
                Kind::Computed(_) => {
                    if !node.dirty {
                        node.dirty = true;
                        stack.extend(node.subscribers.iter().copied());
                    }
                }
                Kind::Effect(_) => {
                    if !node.queued {
                        node.queued = true;
                        self.queue.push_back(sub);
                    }
                }
                Kind::Signal => {}
            }
        }
    }
}

/// Runs `f` with `id` as the active observer, rebuilding its dependencies.
fn run_tracked<R>(id: NodeId, f: impl FnOnce() -> R) -> R {
    let prev = with_rt(|rt| {
        rt.clear_sources(id);
        rt.observer.replace(id)
    });
    // Restore the previous observer even if `f` panics, so a caught panic
    // cannot leave the graph in a corrupted tracking state.
    struct Restore(Option<NodeId>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let prev = self.0;
            with_rt(|rt| rt.observer = prev);
        }
    }
    let _restore = Restore(prev);
    f()
}

fn run_effect(id: NodeId) {
    let f = with_rt(|rt| match &rt.nodes[id].kind {
        Kind::Effect(f) => f.clone(),
        _ => unreachable!("node {id} is not an effect"),
    });
    run_tracked(id, || (f.borrow_mut())());
}

/// Runs queued effects until the graph is stable.
fn flush() {
    let already_flushing = with_rt(|rt| std::mem::replace(&mut rt.flushing, true));
    if already_flushing {
        return;
    }
    let mut iterations = 0;
    loop {
        let next = with_rt(|rt| {
            let id = rt.queue.pop_front()?;
            rt.nodes[id].queued = false;
            Some(id)
        });
        let Some(id) = next else { break };
        iterations += 1;
        if iterations > MAX_FLUSH_ITERATIONS {
            with_rt(|rt| {
                rt.flushing = false;
                rt.queue.clear();
            });
            panic!("pulse: effects did not settle (possible infinite update loop)");
        }
        run_effect(id);
    }
    with_rt(|rt| rt.flushing = false);
}

fn flush_if_idle() {
    if with_rt(|rt| rt.batch_depth == 0) {
        flush();
    }
}

// ---------------------------------------------------------------------------
// Signal
// ---------------------------------------------------------------------------

/// A reactive value that can be read and written.
///
/// Reading with [`Signal::get`] inside a computed or effect subscribes that
/// computation to the signal.
pub struct Signal<T> {
    id: NodeId,
    _ty: PhantomData<fn() -> T>,
}

impl<T> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Signal<T> {}

/// Creates a new signal holding `value`.
pub fn signal<T: 'static>(value: T) -> Signal<T> {
    let id = with_rt(|rt| rt.add(Kind::Signal, Some(Box::new(value)), false));
    Signal { id, _ty: PhantomData }
}

impl<T: Clone + 'static> Signal<T> {
    /// Returns the current value and tracks it as a dependency.
    pub fn get(&self) -> T {
        with_rt(|rt| {
            rt.track(self.id);
            Self::read(rt, self.id)
        })
    }

    /// Returns the current value without tracking it.
    pub fn get_untracked(&self) -> T {
        with_rt(|rt| Self::read(rt, self.id))
    }

    fn read(rt: &Runtime, id: NodeId) -> T {
        rt.nodes[id]
            .value
            .as_ref()
            .expect("pulse: signal read while it is being updated")
            .downcast_ref::<T>()
            .expect("pulse: signal type mismatch")
            .clone()
    }
}

impl<T: PartialEq + 'static> Signal<T> {
    /// Replaces the value. Subscribers are notified only if it changed.
    pub fn set(&self, value: T) {
        let changed = with_rt(|rt| {
            let slot = rt.nodes[self.id].value.as_mut().expect("pulse: signal busy");
            let current = slot.downcast_mut::<T>().expect("pulse: signal type mismatch");
            if *current == value {
                return false;
            }
            *current = value;
            rt.mark_subscribers(self.id);
            true
        });
        if changed {
            flush_if_idle();
        }
    }
}

impl<T: 'static> Signal<T> {
    /// Mutates the value in place and notifies subscribers.
    ///
    /// The closure runs without the runtime borrowed, so it may safely read
    /// *other* signals.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let mut boxed = with_rt(|rt| rt.nodes[self.id].value.take().expect("pulse: signal busy"));
        f(boxed.downcast_mut::<T>().expect("pulse: signal type mismatch"));
        with_rt(|rt| {
            rt.nodes[self.id].value = Some(boxed);
            rt.mark_subscribers(self.id);
        });
        flush_if_idle();
    }
}

// ---------------------------------------------------------------------------
// Computed
// ---------------------------------------------------------------------------

/// A lazily evaluated, cached value derived from other reactive values.
pub struct Computed<T> {
    id: NodeId,
    _ty: PhantomData<fn() -> T>,
}

impl<T> Clone for Computed<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Computed<T> {}

/// Creates a computed value. `f` runs on first read and again on the first
/// read after any of its dependencies changed.
pub fn computed<T: 'static>(f: impl Fn() -> T + 'static) -> Computed<T> {
    let compute: Rc<dyn Fn() -> Box<dyn Any>> = Rc::new(move || Box::new(f()) as Box<dyn Any>);
    let id = with_rt(|rt| rt.add(Kind::Computed(compute), None, true));
    Computed { id, _ty: PhantomData }
}

impl<T: Clone + 'static> Computed<T> {
    /// Returns the (possibly recomputed) value and tracks it as a dependency.
    pub fn get(&self) -> T {
        let id = self.id;
        let compute = with_rt(|rt| {
            rt.track(id);
            match &rt.nodes[id].kind {
                Kind::Computed(f) if rt.nodes[id].dirty => Some(f.clone()),
                _ => None,
            }
        });
        if let Some(compute) = compute {
            let value = run_tracked(id, || compute());
            with_rt(|rt| {
                rt.nodes[id].value = Some(value);
                rt.nodes[id].dirty = false;
            });
        }
        with_rt(|rt| {
            rt.nodes[id]
                .value
                .as_ref()
                .expect("pulse: computed value read during its own computation")
                .downcast_ref::<T>()
                .expect("pulse: computed type mismatch")
                .clone()
        })
    }
}

// ---------------------------------------------------------------------------
// Effects, batching, untracking
// ---------------------------------------------------------------------------

/// Runs `f` and re-runs it whenever anything it read changes.
///
/// The first run happens immediately, unless the effect is created inside a
/// [`batch`] or another effect; then it runs as soon as that work finishes.
pub fn effect(f: impl FnMut() + 'static) {
    let f: Rc<RefCell<dyn FnMut()>> = Rc::new(RefCell::new(f));
    with_rt(|rt| {
        let id = rt.add(Kind::Effect(f), None, false);
        rt.nodes[id].queued = true;
        rt.queue.push_back(id);
    });
    flush_if_idle();
}

/// Groups several writes so dependent effects run once, after `f` returns.
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    with_rt(|rt| rt.batch_depth += 1);
    struct End;
    impl Drop for End {
        fn drop(&mut self) {
            with_rt(|rt| rt.batch_depth -= 1);
        }
    }
    let result = {
        let _end = End;
        f()
    };
    flush_if_idle();
    result
}

/// Runs `f` without tracking any reads as dependencies.
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    let prev = with_rt(|rt| rt.observer.take());
    let result = f();
    with_rt(|rt| rt.observer = prev);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn counter() -> (Rc<Cell<u32>>, impl Fn()) {
        let runs = Rc::new(Cell::new(0));
        let r = runs.clone();
        (runs, move || r.set(r.get() + 1))
    }

    #[test]
    fn signal_get_set() {
        let s = signal(1.0);
        assert_eq!(s.get(), 1.0);
        s.set(2.5);
        assert_eq!(s.get(), 2.5);
        s.update(|v| *v += 1.0);
        assert_eq!(s.get_untracked(), 3.5);
    }

    #[test]
    fn effect_reruns_on_change() {
        let s = signal(0);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen2 = seen.clone();
        effect(move || seen2.borrow_mut().push(s.get()));
        s.set(1);
        s.update(|v| *v += 10);
        assert_eq!(*seen.borrow(), vec![0, 1, 11]);
    }

    #[test]
    fn setting_equal_value_does_not_notify() {
        let s = signal(String::from("a"));
        let (runs, bump) = counter();
        effect(move || {
            s.get();
            bump();
        });
        s.set("a".into());
        assert_eq!(runs.get(), 1);
        s.set("b".into());
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn computed_is_lazy_and_cached() {
        let s = signal(2);
        let (runs, bump) = counter();
        let double = computed(move || {
            bump();
            s.get() * 2
        });
        assert_eq!(runs.get(), 0, "not computed before first read");
        assert_eq!(double.get(), 4);
        assert_eq!(double.get(), 4);
        assert_eq!(runs.get(), 1, "cached between reads");
        s.set(5);
        assert_eq!(runs.get(), 1, "invalidation does not recompute eagerly");
        assert_eq!(double.get(), 10);
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn effect_through_computed_chain() {
        let s = signal(1);
        let a = computed(move || s.get() + 1);
        let b = computed(move || a.get() * 10);
        let out = Rc::new(Cell::new(0));
        let o = out.clone();
        effect(move || o.set(b.get()));
        assert_eq!(out.get(), 20);
        s.set(4);
        assert_eq!(out.get(), 50);
    }

    #[test]
    fn diamond_dependency_runs_effect_once_per_change() {
        let s = signal(1);
        let a = computed(move || s.get() + 1);
        let b = computed(move || s.get() * 2);
        let (runs, bump) = counter();
        let seen = Rc::new(Cell::new(0));
        let seen2 = seen.clone();
        effect(move || {
            bump();
            seen2.set(a.get() + b.get());
        });
        s.set(10);
        assert_eq!(runs.get(), 2);
        assert_eq!(seen.get(), 31);
    }

    #[test]
    fn batch_coalesces_updates() {
        let a = signal(0);
        let b = signal(0);
        let (runs, bump) = counter();
        effect(move || {
            a.get();
            b.get();
            bump();
        });
        let ret = batch(|| {
            a.set(1);
            b.set(2);
            a.set(3);
            "done"
        });
        assert_eq!(ret, "done");
        assert_eq!(runs.get(), 2, "one initial run + one after the batch");
    }

    #[test]
    fn nested_batches_flush_at_outermost_end() {
        let a = signal(0);
        let (runs, bump) = counter();
        effect(move || {
            a.get();
            bump();
        });
        batch(|| {
            a.set(1);
            batch(|| a.set(2));
            assert_eq!(runs.get(), 1);
        });
        assert_eq!(runs.get(), 2);
    }

    #[test]
    fn dynamic_dependencies_are_rebuilt() {
        let use_a = signal(true);
        let a = signal(1);
        let b = signal(100);
        let (runs, bump) = counter();
        effect(move || {
            bump();
            if use_a.get() {
                a.get();
            } else {
                b.get();
            }
        });
        b.set(101);
        assert_eq!(runs.get(), 1, "b is not a dependency yet");
        use_a.set(false);
        assert_eq!(runs.get(), 2);
        a.set(2);
        assert_eq!(runs.get(), 2, "a is no longer a dependency");
        b.set(102);
        assert_eq!(runs.get(), 3);
    }

    #[test]
    fn untrack_skips_dependency() {
        let a = signal(1);
        let (runs, bump) = counter();
        effect(move || {
            untrack(|| a.get());
            bump();
        });
        a.set(2);
        assert_eq!(runs.get(), 1);
    }

    #[test]
    fn effect_may_write_other_signals() {
        let a = signal(1);
        let b = signal(0);
        effect(move || b.set(a.get() * 2));
        let out = Rc::new(Cell::new(0));
        let o = out.clone();
        effect(move || o.set(b.get()));
        a.set(21);
        assert_eq!(out.get(), 42);
    }

    #[test]
    fn update_closure_may_read_other_signals() {
        let a = signal(5);
        let b = signal(1);
        b.update(|v| *v += a.get_untracked());
        assert_eq!(b.get(), 6);
    }

    #[test]
    #[should_panic(expected = "did not settle")]
    fn infinite_loop_is_detected() {
        let a = signal(0);
        effect(move || {
            let v = a.get();
            a.set(v + 1);
        });
    }
}
