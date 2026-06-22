## ADDED Requirements

### Requirement: 图片文件查看器

Files tab 中的图片文件 SHALL 以内嵌预览方式显示，支持缩放和拖拽。

#### Scenario: 打开图片文件
- **WHEN** 用户打开一个图片文件（`.png`、`.jpg`、`.jpeg`、`.gif`、`.webp`、`.svg`）
- **THEN** 在查看器区域居中显示图片
- **AND** 图片初始缩放为 fit-contain（适应查看器区域）
- **AND** 工具栏显示文件名、图片尺寸（宽×高 px）、文件大小

#### Scenario: 缩放操作
- **WHEN** 用户在图片上滚动鼠标滚轮
- **THEN** 图片以鼠标位置为锚点进行缩放
- **AND** 支持 10% - 500% 缩放范围
- **AND** 工具栏显示当前缩放比例

#### Scenario: 拖拽平移
- **WHEN** 图片放大超出查看器区域
- **THEN** 用户可通过鼠标拖拽平移查看不同区域

#### Scenario: 缩放控制按钮
- **WHEN** 用户点击工具栏的"适应窗口"按钮
- **THEN** 图片重置为 fit-contain 缩放
- **AND** 点击"原始大小"按钮时显示为 100% 缩放

#### Scenario: SVG 文件
- **WHEN** 用户打开 `.svg` 文件
- **THEN** 工具栏提供"图片预览"和"源码查看"切换
- **AND** 默认为图片预览模式
- **AND** 源码模式使用 CodeMirror 6 渲染 XML/SVG 代码
