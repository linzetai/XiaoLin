import { useState } from "react";
import { Terminal, Package, Download, Settings, BookOpen, AlertTriangle, CheckCircle, XCircle } from "lucide-react";
import { SectionTitle } from "./SettingsShared";

export function McpInstallGuide() {
  const [activeTab, setActiveTab] = useState<"install" | "configure" | "examples">("install");

  return (
    <div className="space-y-5">
      <SectionTitle>MCP 服务器安装指南</SectionTitle>
      
      <div className="flex rounded-[var(--radius-sm)] p-1" style={{ background: "var(--bg-base)" }}>
        <button
          className={`flex-1 rounded-[var(--radius-xs)] py-2 text-[13px] font-medium transition-colors ${
            activeTab === "install" ? "" : "hover:bg-[var(--bg-hover)]"
          }`}
          style={{
            background: activeTab === "install" ? "var(--fill-primary)" : "transparent",
            color: activeTab === "install" ? "var(--fill-inverse)" : "var(--fill-secondary)"
          }}
          onClick={() => setActiveTab("install")}
        >
          安装 MCP 服务器
        </button>
        <button
          className={`flex-1 rounded-[var(--radius-xs)] py-2 text-[13px] font-medium transition-colors ${
            activeTab === "configure" ? "" : "hover:bg-[var(--bg-hover)]"
          }`}
          style={{
            background: activeTab === "configure" ? "var(--fill-primary)" : "transparent",
            color: activeTab === "configure" ? "var(--fill-inverse)" : "var(--fill-secondary)"
          }}
          onClick={() => setActiveTab("configure")}
        >
          配置 MCP 服务器
        </button>
        <button
          className={`flex-1 rounded-[var(--radius-xs)] py-2 text-[13px] font-medium transition-colors ${
            activeTab === "examples" ? "" : "hover:bg-[var(--bg-hover)]"
          }`}
          style={{
            background: activeTab === "examples" ? "var(--fill-primary)" : "transparent",
            color: activeTab === "examples" ? "var(--fill-inverse)" : "var(--fill-secondary)"
          }}
          onClick={() => setActiveTab("examples")}
        >
          示例服务器
        </button>
      </div>

      {activeTab === "install" && (
        <div className="space-y-4">
          <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-[var(--bg-selected)]">
                <Terminal size={14} style={{ color: "var(--accent)" }} />
              </div>
              <div className="flex-1">
                <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>使用 npm 安装 MCP 服务器</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                  使用 npm 或 yarn 安装 MCP 服务器
                </p>
                <div className="mt-2 rounded-[var(--radius-xs)] p-3 font-mono text-[12px] leading-relaxed" style={{ background: "var(--bg-base)", color: "var(--fill-tertiary)" }}>
                  # 使用 npm<br />
                  npm install -g @modelcontextprotocol/server<br /><br />
                  
                  # 或使用 yarn<br />
                  yarn global add @modelcontextprotocol/server<br /><br />
                  
                  # 或使用 npx 临时运行<br />
                  npx @modelcontextprotocol/server@latest
                </div>
              </div>
            </div>
          </div>

          <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-[var(--bg-selected)]">
                <Package size={14} style={{ color: "var(--accent)" }} />
              </div>
              <div className="flex-1">
                <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>通过包管理器安装</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                  某些 MCP 服务器可通过包管理器安装
                </p>
                <div className="mt-2 rounded-[var(--radius-xs)] p-3 font-mono text-[12px] leading-relaxed" style={{ background: "var(--bg-base)", color: "var(--fill-tertiary)" }}>
                  # 使用 Homebrew (macOS)<br />
                  brew install modelcontextprotocol/tap/mcp-server<br /><br />
                  
                  # 使用 Cargo (Rust)<br />
                  cargo install mcp-server
                </div>
              </div>
            </div>
          </div>

          <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-[var(--bg-selected)]">
                <Download size={14} style={{ color: "var(--accent)" }} />
              </div>
              <div className="flex-1">
                <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>从源码安装</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                  从 GitHub 获取最新的 MCP 服务器实现
                </p>
                <div className="mt-2 rounded-[var(--radius-xs)] p-3 font-mono text-[12px] leading-relaxed" style={{ background: "var(--bg-base)", color: "var(--fill-tertiary)" }}>
                  # 克克隆仓库<br />
                  git clone https://github.com/modelcontextprotocol/servers.git<br />
                  cd servers<br /><br />
                  
                  # 安装依赖并构建<br />
                  npm install<br />
                  npm run build<br /><br />
                  
                  # 启动服务器<br />
                  npm start
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {activeTab === "configure" && (
        <div className="space-y-4">
          <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-[var(--bg-selected)]">
                <Settings size={14} style={{ color: "var(--accent)" }} />
              </div>
              <div className="flex-1">
                <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>配置 FastClaw 中的 MCP 服务器</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                  在 FastClaw 设置中添加 MCP 服务器配置
                </p>
                <div className="mt-3 space-y-2">
                  <div className="flex items-start gap-2 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                    <div className="mt-1 h-1.5 w-1.5 rounded-full" style={{ background: "var(--accent)" }} />
                    <div>ID: 为服务器指定唯一标识符（如 chrome-devtools, github, etc.）</div>
                  </div>
                  <div className="flex items-start gap-2 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                    <div className="mt-1 h-1.5 w-1.5 rounded-full" style={{ background: "var(--accent)" }} />
                    <div>Command: 启动服务器的命令（如 npx, node, python 等）</div>
                  </div>
                  <div className="flex items-start gap-2 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                    <div className="mt-1 h-1.5 w-1.5 rounded-full" style={{ background: "var(--accent)" }} />
                    <div>Args: 传递给命令的参数（如 @modelcontextprotocol/some-server）</div>
                  </div>
                </div>
              </div>
            </div>
          </div>

          <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-[var(--bg-selected)]">
                <Terminal size={14} style={{ color: "var(--accent)" }} />
              </div>
              <div className="flex-1">
                <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>示例配置</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                  常见 MCP 服务器的配置示例
                </p>
                <div className="mt-2 rounded-[var(--radius-xs)] p-3 font-mono text-[12px] leading-relaxed" style={{ background: "var(--bg-base)", color: "var(--fill-tertiary)" }}>
                  ID: chrome-devtools<br />
                  Command: npx<br />
                  Args: @modelcontextprotocol/chrome-devtools-mcp@latest
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      {activeTab === "examples" && (
        <div className="space-y-4">
          <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-[var(--bg-selected)]">
                <BookOpen size={14} style={{ color: "var(--accent)" }} />
              </div>
              <div className="flex-1">
                <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>Chrome DevTools MCP</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                  允许 AI 代理与 Chrome 浏览器进行交互
                </p>
                <div className="mt-2 rounded-[var(--radius-xs)] p-3 font-mono text-[12px] leading-relaxed" style={{ background: "var(--bg-base)", color: "var(--fill-tertiary)" }}>
                  ID: chrome-devtools<br />
                  Command: npx<br />
                  Args: @modelcontextprotocol/chrome-devtools-mcp@latest
                </div>
              </div>
            </div>
          </div>

          <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-[var(--bg-selected)]">
                <BookOpen size={14} style={{ color: "var(--accent)" }} />
              </div>
              <div className="flex-1">
                <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>GitHub MCP</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                  允许 AI 代理与 GitHub 进行交互
                </p>
                <div className="mt-2 rounded-[var(--radius-xs)] p-3 font-mono text-[12px] leading-relaxed" style={{ background: "var(--bg-base)", color: "var(--fill-tertiary)" }}>
                  ID: github<br />
                  Command: npx<br />
                  Args: @modelcontextprotocol/github-mcp@latest
                </div>
              </div>
            </div>
          </div>

          <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "var(--bg-elevated)", border: "0.5px solid var(--separator-opaque)" }}>
            <div className="flex items-start gap-3">
              <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full bg-[var(--bg-selected)]">
                <BookOpen size={14} style={{ color: "var(--accent)" }} />
              </div>
              <div className="flex-1">
                <h3 className="font-medium text-[14px]" style={{ color: "var(--fill-primary)" }}>文件系统 MCP</h3>
                <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
                  允许 AI 代理安全地读写文件
                </p>
                <div className="mt-2 rounded-[var(--radius-xs)] p-3 font-mono text-[12px] leading-relaxed" style={{ background: "var(--bg-base)", color: "var(--fill-tertiary)" }}>
                  ID: filesystem<br />
                  Command: npx<br />
                  Args: @modelcontextprotocol/filesystem-mcp@latest
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

      <div className="rounded-[var(--radius-sm)] p-4" style={{ background: "color-mix(in srgb, var(--blue) 10%, transparent)", border: "0.5px solid var(--separator-opaque)" }}>
        <div className="flex items-start gap-3">
          <div className="mt-0.5 flex h-6 w-6 items-center justify-center rounded-full" style={{ background: "color-mix(in srgb, var(--blue) 20%, transparent)" }}>
            <AlertTriangle size={14} style={{ color: "var(--blue)" }} />
          </div>
          <div>
            <h3 className="font-medium text-[14px]" style={{ color: "var(--blue)" }}>安全提示</h3>
            <p className="mt-1 text-[12px]" style={{ color: "var(--fill-secondary)" }}>
              在配置 MCP 服务器时，请确保只安装和信任来自可信来源的服务器。MCP 服务器可能会访问您的系统资源。
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}