use xiaolin_core::migration::{ExportOptions, ImportOptions, export_data, import_data};
use xiaolin_core::config::ConfigMode;

#[tokio::test]
async fn test_migration_flow() -> Result<(), Box<dyn std::error::Error>> {
    // 设置导出选项
    let export_options = ExportOptions {
        include_sessions: false,  // 为了简化测试，我们不导出会话数据
        include_skills: false,    // 为了简化测试，我们不导出技能
        include_agent_workspaces: false, // 为了简化测试，我们不导出工作目录
    };

    // 执行导出
    let migration_data = export_data(&ConfigMode::from_flags(false, None), &export_options).await?;
    println!("✓ Data exported successfully");

    // 序列化数据
    let serialized_data = xiaolin_core::migration::serialize_migration_data(&migration_data)?;
    println!("✓ Data serialized to {} bytes", serialized_data.len());

    // 反序列化数据
    let deserialized_data = xiaolin_core::migration::deserialize_migration_data(&serialized_data)?;
    println!("✓ Data deserialized successfully");

    // 设置导入选项
    let import_options = ImportOptions {
        merge: false,
        overwrite_config: false,
        overwrite_agents: false,
        overwrite_sessions: false,
        overwrite_skills: false,
    };

    // 执行导入
    import_data(&deserialized_data, &ConfigMode::from_flags(false, None), &import_options).await?;
    println!("✓ Data imported successfully");

    Ok(())
}

#[tokio::test]
async fn test_export_import_with_merge() -> Result<(), Box<dyn std::error::Error>> {
    let export_options = ExportOptions {
        include_sessions: false,
        include_skills: false,
        include_agent_workspaces: false,
    };

    let migration_data = export_data(&ConfigMode::from_flags(false, None), &export_options).await?;
    println!("✓ Data exported for merge test");

    let import_options = ImportOptions {
        merge: true,  // 启用合并
        overwrite_config: false,
        overwrite_agents: false,
        overwrite_sessions: false,
        overwrite_skills: false,
    };

    import_data(&migration_data, &ConfigMode::from_flags(false, None), &import_options).await?;
    println!("✓ Data imported with merge successfully");

    Ok(())
}