// Ambient declarations for non-TypeScript imports Pulse understands.
// (A separate script file: wildcard module declarations cannot live in the
// `pulse` module itself.)

/** `import "./styles.css";` adds the stylesheet to the app's `app.css`. */
declare module "*.css";
