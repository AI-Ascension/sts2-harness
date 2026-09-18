import { afterEach, describe, expect, it, vi } from "vitest";
import type { HarnessToolRegistry, TurnContext } from "@exo/harness";

const mocks = vi.hoisted(() => ({ run: vi.fn(), event: vi.fn() }));
vi.mock("@exo/harness", () => ({
  defineHarness: (definition: unknown) => definition,
  appendCustomEvent: (...args: unknown[]) => mocks.event(...args),
}));
vi.mock("@exo/model-runtime/turn-loop", () => ({
  runResponsesHarnessTurn: (...args: unknown[]) => mocks.run(...args),
}));

import harness from "./index";

type RegisterTools = (tools: HarnessToolRegistry, context: TurnContext) => void;
const FORBIDDEN_NAMES = [
  "shell", "SHELL", "Shell", "functions.shell", "built_in:shell", "exo.shell",
  "install_agent_tool", "uninstall_agent_tool", "manage_tool", "inspect_tools",
  "install_skill", "remember", "sts2_lookup_query",
];
function tool(name: string, reached: string[]) {
  return {
    definition: { name, description: "synthetic", parameters: { type: "object" } },
    source: "built_in" as const,
    handler: { async execute() { reached.push(name); return "reached"; } },
  };
}

const originalFetch = globalThis.fetch;
function context(overrides: Record<string, unknown> = {}): TurnContext {
  return {
    agentConfig: {
      enableAgentToolCreation: false,
      maxToolRoundTrips: 0,
      typescript: { modulePath: "owned", toolModulePaths: [] },
      ...overrides,
    },
    exoharness: { current: { turn: {} } },
  } as unknown as TurnContext;
}

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.unstubAllEnvs();
  vi.clearAllMocks();
});

describe("restricted Exo turn", () => {
  it("rejects installed tools and tool creation before starting the executor turn", async () => {
    await expect(harness.runTurn(context({ enableAgentToolCreation: true })))
      .rejects.toThrow("sts2_restricted_profile_required");
    await expect(harness.runTurn(context({
      typescript: { modulePath: "owned", toolModulePaths: ["foreign"] },
    }))).rejects.toThrow("sts2_restricted_profile_required");
    expect(mocks.run).not.toHaveBeenCalled();
  });

  it("denies a second SDK fetch attempt and records it without forwarding", async () => {
    vi.stubEnv("STS2_EXO_ALLOWED_ENDPOINT", "http://127.0.0.1:12345");
    const forwarded = vi.fn(async () => new Response("{}", { status: 200 }));
    globalThis.fetch = forwarded;
    mocks.run.mockImplementationOnce(async () => {
      await fetch("http://127.0.0.1:12345/responses", { method: "POST", body: "{}" });
      await fetch("http://127.0.0.1:12345/responses", { method: "POST", body: "{}" });
    });
    await expect(harness.runTurn(context())).rejects.toThrow("sts2_model_write_denied");
    expect(forwarded).toHaveBeenCalledTimes(1);
    expect(mocks.event).toHaveBeenCalledWith(expect.anything(), "sts2.exo-fetch-guard-v1", {
      attempts: 2, forwarded: 1, denied: 1,
    });
    expect(globalThis.fetch).toBe(forwarded);
  });

  it("rejects foreign egress and restores fetch even when the turn fails", async () => {
    vi.stubEnv("STS2_EXO_ALLOWED_ENDPOINT", "http://127.0.0.1:12345");
    const forwarded = vi.fn();
    globalThis.fetch = forwarded;
    mocks.run.mockImplementationOnce(async () => {
      await fetch("https://unreviewed.invalid/responses", { method: "POST", body: "{}" });
    });
    await expect(harness.runTurn(context())).rejects.toThrow("sts2_model_write_denied");
    expect(forwarded).not.toHaveBeenCalled();
    expect(globalThis.fetch).toBe(forwarded);
  });

  it("bounds streamed response bytes before upstream parsing", async () => {
    vi.stubEnv("STS2_EXO_ALLOWED_ENDPOINT", "http://127.0.0.1:12345");
    const forwarded = vi.fn(async () => new Response("x".repeat(64 * 1024 + 1)));
    globalThis.fetch = forwarded;
    mocks.run.mockImplementationOnce(async () => {
      await fetch("http://127.0.0.1:12345/responses", { method: "POST", body: "{}" });
    });
    await expect(harness.runTurn(context())).rejects.toThrow("sts2_model_bytes_exceeded");
    expect(forwarded).toHaveBeenCalledTimes(1);
  });

  it("denies shell, install_agent_tool, manage_tool, inspect_tools, install_skill and remember by name at dispatch", async () => {
    vi.stubEnv("STS2_EXO_ALLOWED_ENDPOINT", "http://127.0.0.1:12345");
    globalThis.fetch = vi.fn();
    const { createToolRegistry } = await vi.importActual<typeof import("@exo/harness")>("@exo/harness");
    const reached: string[] = [];
    mocks.run.mockImplementationOnce(async (turn: TurnContext, options: { registerTools: RegisterTools }) => {
      for (const name of FORBIDDEN_NAMES) {
        const tools = createToolRegistry(turn);
        options.registerTools(tools, turn);
        expect(tools.definitions()).toEqual([]);
        expect(() => tools.register(tool(name, reached))).toThrow("sts2_forbidden_tool");
        await expect(tools.executePending([{ toolCallId: `call-${name}`, request: { functionName: name, arguments: {} } }]))
          .rejects.toThrow("sts2_forbidden_tool");
        expect(tools.get(name)).toBeUndefined();
        expect(tools.definitions()).toEqual([]);
      }
    });
    await harness.runTurn(context());
    expect(reached).toEqual([]);
    expect(mocks.event).toHaveBeenCalledWith(expect.anything(), "sts2.exo-tool-guard-v1", {
      registrations: FORBIDDEN_NAMES.length, dispatches: FORBIDDEN_NAMES.length,
    });
  });

  it("rejects a registry that already carries a tool before any handler can run", async () => {
    vi.stubEnv("STS2_EXO_ALLOWED_ENDPOINT", "http://127.0.0.1:12345");
    globalThis.fetch = vi.fn();
    const { createToolRegistry } = await vi.importActual<typeof import("@exo/harness")>("@exo/harness");
    const reached: string[] = [];
    mocks.run.mockImplementationOnce(async (turn: TurnContext, options: { registerTools: RegisterTools }) => {
      const tools = createToolRegistry(turn).register(tool("shell", reached));
      options.registerTools(tools, turn);
    });
    await expect(harness.runTurn(context())).rejects.toThrow("sts2_forbidden_tool");
    expect(reached).toEqual([]);
    expect(mocks.event).toHaveBeenCalledWith(expect.anything(), "sts2.exo-tool-guard-v1", {
      registrations: 1, dispatches: 0,
    });
  });
});
