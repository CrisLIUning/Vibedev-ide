// VibeDev profile for Open Design (https://github.com/nexu-io/open-design)
//
// Drop this file at `~/.open-design/profiles/vibedev.ts` and Open Design's
// daemon (apps/daemon/src/runtimes/local-profiles.ts) will pick it up at
// startup, exposing "VibeDev" alongside the 20 built-in agent definitions
// in its picker. Open Design then drives our ACP agent (claude-code-best /
// `ccb`) for every design Skill it runs.
//
// Why this works as-is:
//   - claude-code-best is a Claude Code fork that preserves the full CLI
//     surface (`-p`, `--input-format stream-json`, `--output-format
//     stream-json`, `--verbose`, `--include-partial-messages`, `--model`,
//     `--add-dir`, `--permission-mode bypassPermissions`, MCP via
//     `--mcp-config` / cwd-local `.mcp.json`). Verified against
//     `ccb -p --help` — 100% argv-compatible with upstream Claude Code.
//   - Open Design's claude.ts adapter therefore works verbatim; we only
//     swap the `id` / `name` / `bin` / `fallbackBins` / `fallbackModels`
//     / `env` fields and inherit everything else.
//   - The `bin` lookup chain matches every shape ccb ships:
//       npm:  `ccb` (cli-node)        ← primary, smallest spawn cost
//       npm:  `claude-code-best`      ← full package name fallback
//       bun:  `ccb-bun` (cli-bun.js)  ← compiled-bun fallback
//
// Models — populated from the live sub2 catalog (the gateway VibeDev's
// account panel + agent both talk to). DEFAULT_MODEL_OPTION first so the
// picker opens on "Default" (the agent itself picks via
// OPENAI_DEFAULT_SONNET_MODEL / _OPUS_MODEL / _HAIKU_MODEL env). The
// explicit entries below are hints for the custom-model dropdown.
//
// Env — mirrors the agent_servers.VibeDev.env block in our Zed-side
// default.json (settings.rs expands ${ZED_BIN_DIR} there; here the env
// is set verbatim, no expansion needed). VIBEDEV_USE_OPENAI=1 makes
// ccb speak the OpenAI-shaped wire format against the sub2 gateway
// instead of going to api.anthropic.com directly. VIBEDEV_CONFIG_DIR
// pins ccb to ~/.vibedev so it doesn't clobber a real Claude Code
// install that happens to be on the same machine.
//
// Maintenance: keep fallbackModels / env in sync with whatever the sub2
// gateway accepts. Source of truth is
// claude-code-best/src/services/api/openai/modelList.ts; if you add a
// new model server-side, drop it in here too.

import { agentCapabilities } from '../capabilities.js';
import { DEFAULT_MODEL_OPTION } from './shared.js';
import type { RuntimeAgentDef } from '../types.js';

export const vibedevAgentDef = {
    id: 'vibedev',
    name: 'VibeDev',
    bin: 'ccb',
    fallbackBins: ['claude-code-best', 'ccb-bun'],
    versionArgs: ['--version'],
    helpArgs: ['-p', '--help'],
    capabilityFlags: {
      // Identical capability probe to claude.ts — we ship the same flags.
      // After `ccb -p --help` returns, Open Design's detection layer sets
      // agentCapabilities['vibedev'][key] = true for each matched flag.
      '--include-partial-messages': 'partialMessages',
      '--add-dir': 'addDir',
    },
    // sub2 gateway model catalog. DEFAULT_MODEL_OPTION first so the
    // picker defaults to "Default" (ccb itself decides via the
    // OPENAI_DEFAULT_*_MODEL env keys below). The labeled entries are
    // hints; users can paste any model id sub2 accepts via Open
    // Design's custom-model input.
    fallbackModels: [
      DEFAULT_MODEL_OPTION,
      { id: 'deepseek-chat',     label: 'DeepSeek Chat (V3)' },
      { id: 'deepseek-reasoner', label: 'DeepSeek Reasoner (R1)' },
      { id: 'deepseek-coder',    label: 'DeepSeek Coder' },
      { id: 'claude-opus-4-7',   label: 'Claude Opus 4.7' },
      { id: 'claude-sonnet-4-6', label: 'Claude Sonnet 4.6' },
      { id: 'claude-haiku-4-5',  label: 'Claude Haiku 4.5' },
      { id: 'gpt-5.5',           label: 'GPT-5.5' },
    ],
    // Identical argv shape to claude.ts. ccb preserves every flag,
    // verified against `ccb -p --help`.
    buildArgs: (_prompt, _imagePaths, extraAllowedDirs = [], options = {}) => {
      const caps = agentCapabilities.get('vibedev') || {};
      const args = [
        '-p',
        '--input-format', 'stream-json',
        '--output-format', 'stream-json',
        '--verbose',
      ];
      if (caps.partialMessages) {
        args.push('--include-partial-messages');
      }
      if (options.model && options.model !== 'default') {
        args.push('--model', options.model);
      }
      const dirs = (extraAllowedDirs || []).filter(
        (d) => typeof d === 'string' && d.length > 0,
      );
      if (dirs.length > 0 && caps.addDir !== false) {
        args.push('--add-dir', ...dirs);
      }
      args.push('--permission-mode', 'bypassPermissions');
      return args;
    },
    promptViaStdin: true,
    promptInputFormat: 'stream-json',
    streamFormat: 'claude-stream-json',
    // ccb auto-loads `.mcp.json` from cwd at spawn (same behaviour as
    // upstream claude). Open Design's `claude-mcp-json` strategy
    // therefore drops the user's external MCP servers in the project
    // cwd before launching us.
    externalMcpInjection: 'claude-mcp-json',
    // Env wiring — mirror of crates/zed/assets/settings/default.json's
    // agent_servers.VibeDev.env. Routes ccb through the sub2 gateway
    // and isolates its config from a parallel Claude Code install.
    env: {
      VIBEDEV_USE_OPENAI: '1',
      // Per-OS config dir. Keep in sync with what VibeDev's Rust side
      // points at: `~/.vibedev` on every platform (`%USERPROFILE%\.vibedev`
      // on Windows). Open Design resolves `~` itself via the daemon's env
      // pipeline, so this works on all three platforms.
      VIBEDEV_CONFIG_DIR: '~/.vibedev',
      OPENAI_DEFAULT_SONNET_MODEL: 'deepseek-chat',
      OPENAI_DEFAULT_OPUS_MODEL:   'deepseek-reasoner',
      OPENAI_DEFAULT_HAIKU_MODEL:  'deepseek-chat',
    },
} satisfies RuntimeAgentDef;
