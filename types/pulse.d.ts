// TypeScript declarations for Pulse.
//
// These let editors and `tsc --noEmit` type-check Pulse apps. The Pulse
// compiler enforces the same rules (a strict subset of TypeScript).

/** A reactive value. Reading `.value` inside JSX, `computed` or `effect` subscribes to it. */
export interface Signal<T> {
  value: T;
}

/** A cached value derived from other signals. Read-only. */
export interface Computed<T> {
  readonly value: T;
}

/** Creates a signal. Supported value types: `number`, `string`, `boolean`. */
export function signal<T extends number | string | boolean>(value: T): Signal<Widen<T>>;

/** Creates a lazily evaluated, cached derived value. */
export function computed<T extends number | string | boolean>(fn: () => T): Computed<Widen<T>>;

/** Runs `fn` now and whenever any signal it read changes. */
export function effect(fn: () => void): void;

/** Applies all writes made in `fn`, then runs dependent effects once. */
export function batch<T>(fn: () => T): T;

/** Runs `fn` without subscribing to the signals it reads. */
export function untrack<T>(fn: () => T): T;

/** `signal(0)` is a `Signal<number>`, not a `Signal<0>`. */
type Widen<T> = T extends number ? number : T extends string ? string : T extends boolean ? boolean : T;

declare global {
  namespace JSX {
    /** A DOM node created by JSX. */
    interface Element {
      readonly __pulseElement: unique symbol;
    }

    type Handler = () => void;
    type Value = string | number;

    /** Attributes accepted on every HTML element. */
    interface HtmlAttributes {
      class?: Value;
      className?: Value;
      id?: Value;
      title?: Value;
      style?: string;
      role?: string;
      tabindex?: Value;
      hidden?: boolean;
      disabled?: boolean;
      checked?: boolean;
      readonly?: boolean;
      required?: boolean;
      href?: string;
      src?: string;
      alt?: string;
      type?: string;
      name?: string;
      value?: Value;
      placeholder?: string;
      for?: string;
      htmlFor?: string;
      key?: Value;
      onClick?: Handler;
      onDblClick?: Handler;
      onInput?: Handler;
      onChange?: Handler;
      onSubmit?: Handler;
      onKeyDown?: Handler;
      onKeyUp?: Handler;
      onFocus?: Handler;
      onBlur?: Handler;
      onMouseEnter?: Handler;
      onMouseLeave?: Handler;
      [attr: `aria-${string}`]: Value | undefined;
      [attr: `data-${string}`]: Value | undefined;
    }

    type IntrinsicElements = {
      [tag in keyof HTMLElementTagNameMap]: HtmlAttributes;
    };

    /** Components take no props yet. */
    interface IntrinsicAttributes {}
  }
}

