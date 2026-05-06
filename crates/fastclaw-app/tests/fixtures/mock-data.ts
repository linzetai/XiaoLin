export const MOCK_AGENTS = [
  { agentId: "assistant", name: "Assistant", model: "gpt-4o", avatar: null },
  { agentId: "coder", name: "Coder", model: "claude-sonnet-4-20250514", avatar: null },
  { agentId: "writer", name: "Writer", model: "gpt-4o-mini", avatar: null },
];

export const MOCK_SESSIONS = [
  {
    id: "sess-001",
    agentId: "assistant",
    title: "测试对话",
    workDir: null,
    messageCount: 2,
    createdAt: "2026-04-30T00:00:00Z",
    updatedAt: "2026-04-30T00:01:00Z",
  },
  {
    id: "sess-002",
    agentId: "coder",
    title: "代码审查",
    workDir: "/home/user/project",
    messageCount: 5,
    createdAt: "2026-04-30T00:00:00Z",
    updatedAt: "2026-04-30T00:02:00Z",
  },
];

export const MOCK_MODELS = [
  { id: "gpt-4o", name: "GPT-4o", provider: "openai_compatible" },
  { id: "claude-sonnet-4-20250514", name: "Claude Sonnet 4", provider: "anthropic" },
  { id: "gpt-4o-mini", name: "GPT-4o Mini", provider: "openai_compatible" },
];

export const MOCK_MESSAGES = [
  {
    id: 1,
    role: "user",
    content: "你好",
    name: null,
    toolCallId: null,
    toolCallsJson: null,
    createdAt: "2026-04-30T00:00:10Z",
  },
  {
    id: 2,
    role: "assistant",
    content: "你好！有什么可以帮你的？",
    name: null,
    toolCallId: null,
    toolCallsJson: null,
    createdAt: "2026-04-30T00:00:12Z",
  },
];

export const MOCK_TOOL_CALL_MESSAGES = [
  {
    id: 1,
    role: "user",
    content: "帮我读取 package.json",
    name: null,
    toolCallId: null,
    toolCallsJson: null,
    createdAt: "2026-04-30T00:00:10Z",
  },
  {
    id: 2,
    role: "assistant",
    content: "我来读取 package.json 文件。",
    name: null,
    toolCallId: null,
    toolCallsJson: [
      {
        id: "tc-001",
        type: "function",
        function: {
          name: "read_file",
          arguments: '{"path": "package.json"}',
        },
      },
    ],
    createdAt: "2026-04-30T00:00:12Z",
  },
  {
    id: 3,
    role: "tool",
    content: '{"name": "fastclaw-app", "version": "0.0.5"}',
    name: "read_file",
    toolCallId: "tc-001",
    toolCallsJson: null,
    createdAt: "2026-04-30T00:00:13Z",
  },
  {
    id: 4,
    role: "assistant",
    content: "package.json 的内容如下：项目名称是 fastclaw-app，版本号 0.0.5。",
    name: null,
    toolCallId: null,
    toolCallsJson: null,
    createdAt: "2026-04-30T00:00:14Z",
  },
];

export const MOCK_STREAMING_CHUNKS = [
  "你好",
  "！我是",
  "FastClaw",
  " 助手",
  "，有什么",
  "可以帮",
  "你的吗？",
];

export function buildHealthResponse() {
  return { status: "ok", version: "0.0.5-test" };
}

export function buildGatewayInfo() {
  return {
    port: 18888,
    wsUrl: "ws://127.0.0.1:18888/ws",
    httpUrl: "http://127.0.0.1:18888",
    version: "test",
  };
}
