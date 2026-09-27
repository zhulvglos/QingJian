# 轻笺

Windows 桌面便签与笔记工具，提供外侧快捷标签、提醒、资讯、双路录音、SenseVoice 本地转写和在线转写纪要。

当前版本：**0.10.3 测试版，Windows x64**。面向小范围试用，尚未完成跨电脑兼容性验证。

## 下载安装

1. 打开 [0.10.3 发布页](https://github.com/zhulvglos/QingJian/releases/tag/v0.10.3)，下载 Assets 中的 `轻笺 V1_0.10.3_x64-setup.exe`。
2. 普通用户无需下载 Source code，无需安装 Git、Node.js、Rust 或 Python。
3. 升级前保存草稿、结束录音，从托盘退出旧程序；重要笔记先通过“设置 → 文件”导出。
4. 运行安装包，选择安装位置，再启动“轻笺 V1”。应用需要 Microsoft Edge WebView2 Runtime；缺少此组件的干净系统尚未实测，请留意安装向导的依赖提示。

仓库若为私有，下载需要相应访问权限。新用户不会继承开发者的密钥、笔记、模型及个人设置。

安装包 SHA-256：

```text
8D063FEC3181B728A3D3052E56DD0FFDFBB1C8FE6443BE17DDDAD0E4FCE36FCA
```

## 首次配置

### 在线文字模型

“设置 → 模型”填写供应商的 Base URL、模型 ID、API Key，保存后点击“测试”。用于转写纪要和模型资讯筛选。密钥使用 Windows 凭据存储，不随源码发布。接口兼容性以实际测试为准；费用由供应商决定。

### 一键安装本地 SenseVoice

1. 在“设置 → 语音”选择自己的**非 C 盘**语音工作目录。
2. 点击“一键检测并安装”，程序下载独立 Python、依赖、SenseVoiceSmall 和语音活动检测模型，不需要自行配置 Anaconda。
3. 等待“可用 · 实际转写通过”，再进入“设置 → 录音”，选择本地 SenseVoice。
4. 下载失败时查看错误并重试；网络不可达时不能保证安装成功。

当前采用 **Windows x64 CPU 方案**，没有 ARM64 原生或 GPU 自动适配。程序检查约 1.5 GB 可用内存和至少 4 GB 可用磁盘；这是安装预检门槛，长录音应预留更多空间。本机测试资源约 3.44 GB，实际占用可能变化。

安装需访问 python.org、PyPI/PythonHosted、PyTorch 下载源及 Hugging Face。安装包不包含模型，完成准备后本地转写可离线运行。**只有 C 盘的电脑目前不能使用本地语音工作目录流程。**

### 在线语音模型

“设置 → 语音”另行填写在线转写的服务地址、模型 ID、API Key，保存并测试。语音模型与文字模型分别配置。选择在线转写或授权本地不可用时在线回退后，音频会发送到所选服务。

## 使用

- 便签／笔记用 Ctrl+S 保存；支持 Ctrl+B、Ctrl+Z、Ctrl+Y。
- 顶部放大镜按标题搜索，支持方向键选择和 Enter 打开。
- 拖动列表卡片固定手柄到侧边快捷标签，点击可打开原记录。
- 待办／已完成依据提醒状态，与正文复选框独立。
- 录音支持麦克风与系统声音、暂停继续、转写校对、纪要、保存到笔记、音频目录定位及历史删除。
- 关闭窗口通常进入托盘，完全退出请使用托盘菜单。

详见 [录音使用说明](AUDIO_WORKFLOW_GUIDE.md)。

## 数据与备份

数据库：`%LOCALAPPDATA%\SHUSHIN\Qingjian\qingjian-v1.sqlite3`。语音资源和录音使用所选目录，部分界面设置保存在 WebView 存储。

“设置 → 文件”支持便签／笔记多选导入导出。备份**不包含原始录音及录音会话历史**；重要音频请单独保存，重要转写和纪要先保存为笔记再备份。不要向仓库提交数据库或凭据。

## 验证范围与已知限制

- 本机隔离环境已完成全新语音资源下载、安装、样例转写、约28秒双路录音转写和重启再次转写。
- 独立验收阶段报告同机测试账户覆盖升级通过；部分功能证据为后端检查，不等于全部界面流程验证。
- 干净 Windows、其他电脑、真实断网完整回退、两小时录音、磁盘耗尽及当前安装版全部语音回归尚未完整验证。
- 安装成功后旧进度文字可能残留，重进页面可刷新；同音词识别可能有误，请校对。
- 详细说明见 [0.10.3 发布记录](docs/releases/v0.10.3.md)。

请通过 [Issues](https://github.com/zhulvglos/QingJian/issues) 提供版本、系统、复现步骤和截图，不上传密钥或私人录音。

## 开发者构建

需要 Windows x64、Node.js（建议22.12+）、Rust MSVC 工具链、Visual Studio C++ Build Tools、Windows SDK 和 WebView2 Runtime。

```powershell
git clone https://github.com/zhulvglos/QingJian.git
cd QingJian
git checkout v0.10.3
npm ci
npm run tauri -- build
```

默认安装包位于 `src-tauri/target/release/bundle/nsis`。可将 CARGO_HOME、RUSTUP_HOME、CARGO_TARGET_DIR、npm 缓存和 TEMP/TMP 指向自选数据盘；无需开发者的 `_kaifa/env.ps1`。依赖使用已提交的锁文件。

Release 使用已归档二进制，不因文档整理重新构建，不承诺重新构建逐字节一致。第三方组件及模型遵循各自许可；本次未新增项目开源许可授权。
