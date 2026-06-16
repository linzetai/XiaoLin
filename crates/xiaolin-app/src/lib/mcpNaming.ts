const MCP_DELIMITER = "__";
const MCP_PREFIX = `mcp${MCP_DELIMITER}`;

/**
 * Replace characters invalid for LLM tool-name APIs with underscores.
 * Only `[a-zA-Z0-9_-]` are kept.
 */
export function sanitizeForApi(name: string): string {
  return name.replace(/[^a-zA-Z0-9_-]/g, "_");
}

export function mcpServerPrefix(serverId: string): string {
  return `${MCP_PREFIX}${sanitizeForApi(serverId)}${MCP_DELIMITER}`;
}

export function mcpToolName(serverId: string, toolName: string): string {
  return `${MCP_PREFIX}${sanitizeForApi(serverId)}${MCP_DELIMITER}${sanitizeForApi(toolName)}`;
}

/**
 * Parse `mcp__{serverId}__{toolName}` → `{ serverId, toolName }`.
 * Returns null if the name doesn't match the MCP convention.
 */
export function parseMcpToolName(
  fullName: string,
): { serverId: string; toolName: string } | null {
  if (!fullName.startsWith(MCP_PREFIX)) return null;
  const rest = fullName.slice(MCP_PREFIX.length);
  const idx = rest.indexOf(MCP_DELIMITER);
  if (idx <= 0) return null;
  const serverId = rest.slice(0, idx);
  const toolName = rest.slice(idx + MCP_DELIMITER.length);
  if (!serverId || !toolName) return null;
  return { serverId, toolName };
}

export function isMcpTool(name: string): boolean {
  return parseMcpToolName(name) !== null;
}
