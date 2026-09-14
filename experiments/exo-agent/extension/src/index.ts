import {
  defineHarness,
  type Message,
  type TurnContext,
} from "@exo/harness";
import { runResponsesHarnessTurn } from "@exo/model-runtime/turn-loop";

/**
 * Minimal selected-path reproducer for the reviewed candidate package.
 *
 * This file is intentionally not built by the Rust workspace. An operator with the exact
 * candidate package and model binding can load it at `agent.typescript.module_path` to verify
 * that the owned extension reaches `ResponsesRuntime.runTurn` and `complete`/`completeStream`.
 * The repository cannot claim a terminal STS2 decision or native bridge result until that spike
 * is run with the real package, model, bridge, and licensed host.
 */
export default defineHarness({
  async runTurn(context: TurnContext): Promise<void> {
    await runResponsesHarnessTurn(context, {
      instructions: syntheticInstructions,
      registerTools: () => undefined,
    });
  },
});

const syntheticInstructions = (): Message[] => [
  {
    role: "developer",
    content:
      "Synthetic STS2 bridge spike: return one bounded terminal decision object and no tool call.",
  },
];
