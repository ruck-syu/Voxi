import { spawn } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";
import { existsSync } from "node:fs";

const modelDir = process.env.VOXI_PLAN_MODEL_DIR ?? join(homedir(), ".local/share/voxi/experiments/plan-model-v4c-2026-11-17/native-model");
for (const file of ["config.json", "tokenizer.json", "model-q8_0.gguf"]) {
  if (!existsSync(join(modelDir, file))) throw new Error(`Missing trained model: ${join(modelDir, file)}`);
}
const child = spawn("npm", ["run", "tauri", "--", "dev", "--features", "tiny-model"], {
  stdio: "inherit",
  env: { ...process.env, VOXI_PLAN_MODEL_DIR: modelDir },
});
for (const signal of ["SIGINT", "SIGTERM"]) process.on(signal, () => child.kill(signal));
child.on("exit", code => process.exit(code ?? 1));
