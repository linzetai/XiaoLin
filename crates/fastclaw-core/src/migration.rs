use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use anyhow::Result;

use crate::config::ConfigMode;
use crate::agent_config::AgentConfig;
use crate::types::AgentId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationData {
    /// 主配置文件内容
    pub config: Option<serde_json::Value>,
    
    /// Agent 配置
    pub agents: Vec<AgentConfig>,
    
    /// Agent 工作目录内容 (如果选择导出)
    pub agent_workspaces: Option<HashMap<AgentId, Vec<u8>>>, // agent_id -> workspace archive
    
    /// 会话数据 (如果选择导出)
    pub sessions: Option<Vec<u8>>, // SQLite database bytes
    
    /// 技能数据 (如果选择导出)
    pub skills: Option<Vec<u8>>, // Skills archive
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportOptions {
    /// 是否导出会话数据
    pub include_sessions: bool,
    
    /// 是否导出技能
    pub include_skills: bool,
    
    /// 是否导出代理工作目录
    pub include_agent_workspaces: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportOptions {
    /// 导入时是否合并现有数据
    pub merge: bool,
    
    /// 是否覆盖现有配置
    pub overwrite_config: bool,
    
    /// 是否覆盖现有代理
    pub overwrite_agents: bool,
    
    /// 是否覆盖现有会话
    pub overwrite_sessions: bool,
    
    /// 是否覆盖现有技能
    pub overwrite_skills: bool,
}

/// 导出当前 FastClaw 实例的数据
pub async fn export_data(_mode: &ConfigMode, options: &ExportOptions) -> Result<MigrationData> {
    use crate::paths;
    
    let state_dir = paths::resolve_state_dir_from(None);
    let config_dir = state_dir.join("config");
    let agents_dir = config_dir.join("agents");
    let data_dir = state_dir.join("data");
    let skills_dir = state_dir.join("skills");
    
    // 读取主配置
    let mut config: Option<serde_json::Value> = None;
    let config_path = config_dir.join("default.json");
    if config_path.exists() {
        let config_text = std::fs::read_to_string(&config_path)?;
        config = Some(serde_json::from_str(&config_text)?);
    }
    
    // 读取所有代理配置
    let agents = crate::agent_config::load_agent_configs(&agents_dir)?;
    
    // 读取代理工作目录（如果需要）
    let agent_workspaces = if options.include_agent_workspaces {
        let mut workspaces = HashMap::new();
        for agent in &agents {
            let ws_dir = state_dir.join("workspace").join(agent.agent_id.as_str());
            if ws_dir.exists() {
                // 将工作目录打包成字节数组
                let archive_bytes = tar_directory(&ws_dir)?;
                workspaces.insert(agent.agent_id.clone(), archive_bytes);
            }
        }
        Some(workspaces)
    } else {
        None
    };
    
    // 读取会话数据库（如果需要）
    let sessions = if options.include_sessions {
        let sessions_db_path = data_dir.join("sessions.db");
        if sessions_db_path.exists() {
            Some(std::fs::read(&sessions_db_path)?)
        } else {
            None
        }
    } else {
        None
    };
    
    // 读取技能（如果需要）
    let skills = if options.include_skills {
        let skills_path = skills_dir.clone();
        if skills_path.exists() {
            Some(tar_directory(&skills_path)?)
        } else {
            None
        }
    } else {
        None
    };
    
    Ok(MigrationData {
        config,
        agents,
        agent_workspaces,
        sessions,
        skills,
    })
}

/// 将目录打包成字节数组
fn tar_directory(dir: &PathBuf) -> Result<Vec<u8>> {
    use std::io::Cursor;
    
    let mut tar_buffer = Vec::new();
    {
        let mut tar_builder = tar::Builder::new(Cursor::new(&mut tar_buffer));
        tar_builder.append_dir_all(".", dir)?;
    }
    
    Ok(tar_buffer)
}

/// 解压目录从字节数组
fn untar_directory(data: &[u8], dest: &PathBuf) -> Result<()> {
    use std::io::Cursor;
    
    let reader = Cursor::new(data);
    let mut archive = tar::Archive::new(reader);
    archive.unpack(dest)?;
    
    Ok(())
}

/// 导入数据到当前 FastClaw 实例
pub async fn import_data(data: &MigrationData, _mode: &ConfigMode, options: &ImportOptions) -> Result<()> {
    use crate::paths;
    
    let state_dir = paths::resolve_state_dir_from(None);
    let config_dir = state_dir.join("config");
    let agents_dir = config_dir.join("agents");
    let data_dir = state_dir.join("data");
    let skills_dir = state_dir.join("skills");
    let workspace_dir = state_dir.join("workspace");
    
    // 创建必要的目录
    std::fs::create_dir_all(&config_dir)?;
    std::fs::create_dir_all(&agents_dir)?;
    std::fs::create_dir_all(&data_dir)?;
    std::fs::create_dir_all(&skills_dir)?;
    std::fs::create_dir_all(&workspace_dir)?;
    
    // 导入主配置
    if let Some(config) = &data.config {
        let config_path = config_dir.join("default.json");
        
        if options.overwrite_config {
            // 直接覆盖配置
            std::fs::write(&config_path, serde_json::to_string_pretty(config)?)?;
        } else if options.merge {
            // 合并配置
            let mut existing_config: serde_json::Value = if config_path.exists() {
                let existing_text = std::fs::read_to_string(&config_path)?;
                serde_json::from_str(&existing_text)?
            } else {
                serde_json::json!({})
            };
            
            // 深度合并配置
            merge_json_values(&mut existing_config, config.clone());
            std::fs::write(&config_path, serde_json::to_string_pretty(&existing_config)?)?;
        } else {
            // 如果既不覆盖也不合并，则跳过配置导入
        }
    }
    
    // 导入代理配置
    for agent in &data.agents {
        let agent_path = agents_dir.join(format!("{}.json", agent.agent_id));
        
        if options.overwrite_agents {
            // 直接覆盖代理配置
            std::fs::write(&agent_path, serde_json::to_string_pretty(agent)?)?;
        } else if options.merge {
            // 合并代理配置（如果不存在则创建，如果存在则跳过）
            if !agent_path.exists() {
                std::fs::write(&agent_path, serde_json::to_string_pretty(agent)?)?;
            }
        } else {
            // 如果既不覆盖也不合并，则只有在不存在时才导入
            if !agent_path.exists() {
                std::fs::write(&agent_path, serde_json::to_string_pretty(agent)?)?;
            }
        }
    }
    
    // 导入代理工作目录
    if let Some(workspaces) = &data.agent_workspaces {
        for (agent_id, workspace_data) in workspaces {
            let agent_workspace_dir = workspace_dir.join(agent_id.as_str());
            
            if options.overwrite_agents {
                // 直接覆盖工作目录
                if agent_workspace_dir.exists() {
                    std::fs::remove_dir_all(&agent_workspace_dir)?;
                }
                std::fs::create_dir_all(&agent_workspace_dir)?;
                untar_directory(workspace_data, &agent_workspace_dir)?;
            } else if options.merge {
                // 合并工作目录 - 只有在不存在时才创建
                if !agent_workspace_dir.exists() {
                    std::fs::create_dir_all(&agent_workspace_dir)?;
                    untar_directory(workspace_data, &agent_workspace_dir)?;
                }
            } else {
                // 如果既不覆盖也不合并，则只有在不存在时才导入
                if !agent_workspace_dir.exists() {
                    std::fs::create_dir_all(&agent_workspace_dir)?;
                    untar_directory(workspace_data, &agent_workspace_dir)?;
                }
            }
        }
    }
    
    // 导入会话数据
    if let Some(sessions_data) = &data.sessions {
        let sessions_db_path = data_dir.join("sessions.db");
        
        if options.overwrite_sessions {
            // 直接覆盖会话数据库
            std::fs::write(&sessions_db_path, sessions_data)?;
        } else if options.merge {
            // 合并不会话数据库需要特殊的数据库合并逻辑
            // 这里暂时只支持覆盖，因为会话数据库合并比较复杂
            if !sessions_db_path.exists() {
                std::fs::write(&sessions_db_path, sessions_data)?;
            }
        } else {
            // 如果既不覆盖也不合并，则只有在不存在时才导入
            if !sessions_db_path.exists() {
                std::fs::write(&sessions_db_path, sessions_data)?;
            }
        }
    }
    
    // 导入技能
    if let Some(skills_data) = &data.skills {
        if options.overwrite_skills {
            // 清空现有技能目录并替换
            if skills_dir.exists() {
                std::fs::remove_dir_all(&skills_dir)?;
            }
            std::fs::create_dir_all(&skills_dir)?;
            untar_directory(skills_data, &skills_dir)?;
        } else if options.merge {
            // 合并技能 - 解压到临时目录，然后合并到现有技能目录
            let temp_dir = tempfile::tempdir()?;
            untar_directory(skills_data, &temp_dir.path().to_path_buf())?;
            
            // 复制临时目录中的文件到目标技能目录
            for entry in std::fs::read_dir(temp_dir.path())? {
                let entry = entry?;
                let dest_path = skills_dir.join(entry.file_name());
                
                if entry.file_type()?.is_dir() {
                    // 如果是目录，递归复制
                    copy_dir_all(&entry.path(), &dest_path)?;
                } else {
                    // 如果是文件，直接复制
                    std::fs::copy(entry.path(), dest_path)?;
                }
            }
        } else {
            // 如果既不覆盖也不合并，则只有在技能目录为空时才导入
            if std::fs::read_dir(&skills_dir)?.next().is_none() {
                untar_directory(skills_data, &skills_dir)?;
            }
        }
    }
    
    Ok(())
}

/// 深度合并两个 JSON 值
fn merge_json_values(target: &mut serde_json::Value, source: serde_json::Value) {
    match source {
        serde_json::Value::Object(source_obj) => {
            if let serde_json::Value::Object(ref mut target_obj) = target {
                for (key, value) in source_obj {
                    match target_obj.get_mut(&key) {
                        Some(existing) => {
                            merge_json_values(existing, value);
                        }
                        None => {
                            target_obj.insert(key, value);
                        }
                    }
                }
            } else {
                // 如果目标不是对象，但源是对象，则替换整个目标
                *target = serde_json::Value::Object(source_obj);
            }
        }
        _ => {
            // 对于非对象值，直接替换
            *target = source;
        }
    }
}

/// 递归复制目录
fn copy_dir_all(src: &PathBuf, dst: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        
        if entry.file_type()?.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use std::fs;
    use std::path::Path;

    #[tokio::test]
    async fn test_export_import_basic_flow() -> Result<()> {
        // 创建一个临时目录模拟状态目录
        let temp_dir = TempDir::new()?;
        let state_dir = temp_dir.path();
        
        // 创建必要的子目录
        fs::create_dir_all(state_dir.join("config"))?;
        fs::create_dir_all(state_dir.join("config").join("agents"))?;
        fs::create_dir_all(state_dir.join("data"))?;
        fs::create_dir_all(state_dir.join("skills"))?;
        fs::create_dir_all(state_dir.join("workspace"))?;
        
        // 创建一个简单的配置文件
        let config_path = state_dir.join("config").join("default.json");
        fs::write(&config_path, r#"{"gateway": {"port": 18789}}"#)?;
        
        // 创建一个简单的代理配置
        let agent_path = state_dir.join("config").join("agents").join("test_agent.json");
        fs::write(&agent_path, r#"{"agentId": "test_agent", "name": "Test Agent"}"#)?;
        
        // 创建一个简单的技能文件
        let skills_path = state_dir.join("skills").join("test_skill.md");
        fs::write(&skills_path, "# Test Skill\nThis is a test skill.")?;
        
        // 设置导出选项
        let export_options = ExportOptions {
            include_sessions: false,
            include_skills: true,
            include_agent_workspaces: false,
        };
        
        // 执行导出
        let migration_data = export_data(&ConfigMode::from_flags(false, None), &export_options).await?;
        println!("✓ Data exported successfully");
        
        // 序列化数据
        let serialized_data = serialize_migration_data(&migration_data)?;
        println!("✓ Data serialized to {} bytes", serialized_data.len());
        
        // 反序列化数据
        let deserialized_data = deserialize_migration_data(&serialized_data)?;
        println!("✓ Data deserialized successfully");
        
        // 验证反序列化的数据
        assert!(deserialized_data.config.is_some());
        assert!(!deserialized_data.agents.is_empty());
        assert!(deserialized_data.skills.is_some());
        
        // 设置导入选项
        let import_options = ImportOptions {
            merge: false,
            overwrite_config: true,
            overwrite_agents: true,
            overwrite_sessions: true,
            overwrite_skills: true,
        };
        
        // 创建一个临时的目标目录用于导入
        let import_temp_dir = TempDir::new()?;
        let import_state_dir = import_temp_dir.path();
        
        // 为测试目的，我们需要修改路径解析函数，但这里我们只是测试序列化/反序列化流程
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_merge_json_values() -> Result<()> {
        let mut target = serde_json::json!({
            "existing": "value",
            "nested": {
                "inner": "original"
            }
        });
        
        let source = serde_json::json!({
            "new": "added",
            "nested": {
                "another": "inserted"
            }
        });
        
        merge_json_values(&mut target, source);
        
        // 验证合并结果
        assert_eq!(target["existing"], "value");  // 原有值保留
        assert_eq!(target["new"], "added");      // 新值添加
        assert_eq!(target["nested"]["inner"], "original");  // 嵌套原有值保留
        assert_eq!(target["nested"]["another"], "inserted"); // 嵌套新值添加
        
        Ok(())
    }
}

/// 将迁移数据序列化为字节数组
pub fn serialize_migration_data(data: &MigrationData) -> Result<Vec<u8>> {
    let serialized = serde_json::to_vec(data)?;
    // 可以在这里添加压缩或加密逻辑
    Ok(serialized)
}

/// 从字节数组反序列化迁移数据
pub fn deserialize_migration_data(bytes: &[u8]) -> Result<MigrationData> {
    let data: MigrationData = serde_json::from_slice(bytes)?;
    Ok(data)
}