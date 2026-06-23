## ADDED Requirements

### Requirement: 下载检测和保存（Tier 1）
系统 SHALL 通过 Tauri `on_download` 回调检测下载事件并将文件保存到用户指定目录。

#### Scenario: 检测下载
- **WHEN** Browser WebView 中触发文件下载（点击下载链接、JS 触发等）
- **THEN** `DownloadEvent::Requested` 回调触发
- **AND** 文件保存到默认下载目录（首次使用时提示用户选择）

#### Scenario: 下载完成
- **WHEN** 文件下载完成
- **THEN** `DownloadEvent::Finished` 回调触发
- **AND** 通过 Tauri Event 通知前端

#### Scenario: 下载失败
- **WHEN** 下载过程中出错（网络中断、磁盘满等）
- **THEN** `DownloadEvent::Finished` 回调 `success=false`
- **AND** 通知前端显示失败状态

### Requirement: 下载通知 UI（Tier 1）
系统 SHALL 在 Browser Panel 底部显示下载通知栏。

#### Scenario: 下载进行中
- **WHEN** 有文件正在下载
- **THEN** 底部通知栏显示文件名 + "下载中..." + [取消] 按钮

#### Scenario: 下载完成
- **WHEN** 文件下载完成
- **THEN** 通知栏显示 ✅ + 文件名 + [打开文件] + [打开目录] 按钮

#### Scenario: 下载失败
- **WHEN** 文件下载失败
- **THEN** 通知栏显示 ❌ + 文件名 + 错误信息 + [重试] 按钮

#### Scenario: 多文件下载
- **WHEN** 同时有多个文件下载
- **THEN** 通知栏堆叠显示每个文件的状态

#### Scenario: 通知栏自动消失
- **WHEN** 下载完成且用户无操作
- **THEN** 10 秒后通知栏自动收起（但可从下载历史中查看）

### Requirement: 下载目录配置
系统 SHALL 允许用户配置默认下载目录。

#### Scenario: 首次下载
- **WHEN** 用户首次触发下载且未配置下载目录
- **THEN** 弹出系统文件选择对话框让用户选择目录

#### Scenario: 已配置目录
- **WHEN** 用户已配置下载目录
- **THEN** 文件直接保存到该目录

#### Scenario: 修改下载目录
- **WHEN** 用户在设置中修改下载目录
- **THEN** 后续下载使用新目录
