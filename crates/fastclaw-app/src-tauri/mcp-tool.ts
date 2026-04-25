import { Command } from 'commander';
import { spawn } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

const program = new Command();

// MCP 服务器配置接口
interface McpServerConfig {
  id: string;
  command: string;
  args: string[];
  enabled?: boolean;
}

// 读取配置文件
function readConfig(): McpServerConfig[] {
  const configPath = path.join(process.env.HOME || '', '.fastclaw', 'config', 'default.json');
  if (!fs.existsSync(configPath)) {
    return [];
  }
  
  const configContent = fs.readFileSync(configPath, 'utf-8');
  try {
    const config = JSON.parse(configContent);
    return config.mcpServers || [];
  } catch (error) {
    console.error('Failed to parse config file:', error);
    return [];
  }
}

// 写入配置文件
function writeConfig(servers: McpServerConfig[]): void {
  const configPath = path.join(process.env.HOME || '', '.fastclaw', 'config', 'default.json');
  const configDir = path.dirname(configPath);
  
  if (!fs.existsSync(configDir)) {
    fs.mkdirSync(configDir, { recursive: true });
  }
  
  let config = {};
  if (fs.existsSync(configPath)) {
    const existingContent = fs.readFileSync(configPath, 'utf-8');
    try {
      config = JSON.parse(existingContent);
    } catch (error) {
      console.warn('Failed to parse existing config, starting fresh');
    }
  }
  
  config['mcpServers'] = servers;
  fs.writeFileSync(configPath, JSON.stringify(config, null, 2));
}

// 启动 MCP 服务器
function startMcpServer(config: McpServerConfig): Promise<void> {
  return new Promise((resolve, reject) => {
    if (!config.enabled) {
      console.log(`Skipping disabled server: ${config.id}`);
      resolve();
      return;
    }

    console.log(`Starting MCP server: ${config.id}`);
    const child = spawn(config.command, config.args, { stdio: 'inherit' });

    child.on('error', (error) => {
      console.error(`Failed to start server ${config.id}:`, error.message);
      reject(error);
    });

    child.on('close', (code) => {
      if (code === 0) {
        console.log(`Server ${config.id} exited normally`);
        resolve();
      } else {
        console.error(`Server ${config.id} exited with code ${code}`);
        reject(new Error(`Server exited with code ${code}`));
      }
    });
  });
}

// 列表所有 MCP 服务器
program
  .command('list')
  .description('列出所有配置的 MCP 服务器')
  .action(() => {
    const servers = readConfig();
    if (servers.length === 0) {
      console.log('No MCP servers configured.');
      return;
    }

    console.log('Configured MCP Servers:');
    servers.forEach(server => {
      const status = server.enabled ? 'enabled' : 'disabled';
      console.log(`  - ${server.id} (${status}): ${server.command} ${server.args.join(' ')}`);
    });
  });

// 添加 MCP 服务器
program
  .command('add')
  .description('添加新的 MCP 服务器')
  .argument('<id>', '服务器 ID')
  .argument('<command>', '启动命令')
  .argument('[args...]', '命令参数')
  .option('-d, --disabled', '初始状态为禁用')
  .action((id, command, args, options) => {
    const servers = readConfig();
    
    // 检查是否已存在同名服务器
    if (servers.some(s => s.id === id)) {
      console.error(`Server with ID "${id}" already exists.`);
      process.exit(1);
    }
    
    const newServer: McpServerConfig = {
      id,
      command,
      args: args || [],
      enabled: !options.disabled
    };
    
    servers.push(newServer);
    writeConfig(servers);
    console.log(`Added MCP server: ${id}`);
  });

// 移除 MCP 服务器
program
  .command('remove')
  .alias('rm')
  .description('移除 MCP 服务器')
  .argument('<id>', '服务器 ID')
  .action((id) => {
    const servers = readConfig();
    const initialLength = servers.length;
    const filteredServers = servers.filter(s => s.id !== id);
    
    if (filteredServers.length === initialLength) {
      console.error(`No server found with ID: ${id}`);
      process.exit(1);
    }
    
    writeConfig(filteredServers);
    console.log(`Removed MCP server: ${id}`);
  });

// 启用 MCP 服务器
program
  .command('enable')
  .description('启用 MCP 服务器')
  .argument('<id>', '服务器 ID')
  .action((id) => {
    const servers = readConfig();
    const server = servers.find(s => s.id === id);
    
    if (!server) {
      console.error(`No server found with ID: ${id}`);
      process.exit(1);
    }
    
    if (server.enabled) {
      console.log(`Server ${id} is already enabled`);
      return;
    }
    
    server.enabled = true;
    writeConfig(servers);
    console.log(`Enabled MCP server: ${id}`);
  });

// 禁用 MCP 服务器
program
  .command('disable')
  .description('禁用 MCP 服务器')
  .argument('<id>', '服务器 ID')
  .action((id) => {
    const servers = readConfig();
    const server = servers.find(s => s.id === id);
    
    if (!server) {
      console.error(`No server found with ID: ${id}`);
      process.exit(1);
    }
    
    if (!server.enabled) {
      console.log(`Server ${id} is already disabled`);
      return;
    }
    
    server.enabled = false;
    writeConfig(servers);
    console.log(`Disabled MCP server: ${id}`);
  });

// 启动 MCP 服务器
program
  .command('start')
  .description('启动指定的 MCP 服务器')
  .argument('<id>', '服务器 ID')
  .action(async (id) => {
    const servers = readConfig();
    const server = servers.find(s => s.id === id);
    
    if (!server) {
      console.error(`No server found with ID: ${id}`);
      process.exit(1);
    }
    
    if (!server.enabled) {
      console.log(`Server ${id} is disabled. Enabling it temporarily...`);
    }
    
    try {
      await startMcpServer(server);
      console.log(`Successfully started server: ${id}`);
    } catch (error) {
      console.error(`Failed to start server ${id}:`, error.message);
      process.exit(1);
    }
  });

// 安装常见的 MCP 服务器
program
  .command('install-common')
  .description('安装常见的 MCP 服务器')
  .action(() => {
    console.log('Installing common MCP servers...');
    console.log('1. Chrome DevTools MCP: npx @modelcontextprotocol/chrome-devtools-mcp');
    console.log('2. GitHub MCP: npx @modelcontextprotocol/github-mcp');
    console.log('3. Filesystem MCP: npx @modelcontextprotocol/filesystem-mcp');
    console.log('');
    console.log('To install, run: npm install -g @modelcontextprotocol/[package-name]');
  });

program
  .name('fastclaw-mcp')
  .description('FastClaw MCP Server Management Tool')
  .version('1.0.0');

program.parse();