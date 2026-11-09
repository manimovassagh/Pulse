import { spawn, execFileSync, type ChildProcess } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
export const pulseBin = join(repoRoot, "target/debug/pulse");

/** Runs a `pulse` command to completion (e.g. `build`). */
export function pulse(args: string[], cwd: string = repoRoot): void {
  execFileSync(pulseBin, args, { cwd, stdio: "pipe" });
}

/** A long-running `pulse dev` / `pulse serve` process. */
export interface PulseServer {
  url: string;
  /** Everything the process wrote to stderr so far. */
  log(): string;
  stop(): Promise<void>;
}

/**
 * Starts `pulse <args>` and resolves once it reports the URL it listens on.
 * Port 0 is not used: pulse picks the next free port from the one given, so
 * each caller passes its own base port to avoid collisions between workers.
 */
export function startServer(args: string[], cwd: string): Promise<PulseServer> {
  const child: ChildProcess = spawn(pulseBin, args, { cwd, stdio: ["ignore", "ignore", "pipe"] });
  let output = "";
  return new Promise((resolvePromise, reject) => {
    const timer = setTimeout(() => reject(new Error(`pulse did not start:\n${output}`)), 30_000);
    child.stderr!.on("data", (chunk: Buffer) => {
      output += chunk.toString();
      const match = output.match(/http:\/\/localhost:(\d+)\//);
      if (match) {
        clearTimeout(timer);
        resolvePromise({
          url: match[0],
          log: () => output,
          stop: () =>
            new Promise<void>((done) => {
              if (child.exitCode !== null) return done();
              child.once("exit", () => done());
              child.kill();
            }),
        });
      }
    });
    child.once("exit", (code) => {
      clearTimeout(timer);
      reject(new Error(`pulse exited with code ${code}:\n${output}`));
    });
  });
}

/**
 * Copies an example app into a fresh temporary directory, outside the
 * repository, like a standalone project. Examples with a package.json get the
 * repository's node_modules linked in, standing in for `npm install`.
 */
export function copyExample(name: string): { dir: string; cleanup(): void } {
  const source = join(repoRoot, "examples", name);
  const dir = mkdtempSync(join(tmpdir(), `pulse-${name}-`));
  cpSync(source, dir, {
    recursive: true,
    filter: (src) => !/[\\/](dist|\.pulse|node_modules)$/.test(src),
  });
  if (existsSync(join(source, "package.json"))) {
    symlinkSync(join(repoRoot, "node_modules"), join(dir, "node_modules"), "dir");
  }
  return { dir, cleanup: () => rmSync(dir, { recursive: true, force: true }) };
}
