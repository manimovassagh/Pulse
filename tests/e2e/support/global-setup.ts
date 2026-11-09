import { execFileSync } from "node:child_process";
import { repoRoot } from "./pulse";

/** Compiles the `pulse` CLI once before any spec runs. */
export default function globalSetup(): void {
  execFileSync("cargo", ["build", "--quiet", "-p", "pulse-cli"], { cwd: repoRoot, stdio: "inherit" });
}
