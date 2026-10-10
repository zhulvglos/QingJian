# 云同步验收脚本

`native_voice_reuse.mjs` 验证登录工作区的本机语音资源复用。使用全新 D 盘隔离目录 `QJ_TEST_ROOT`、构建程序 `QJ_TEST_EXE`、本机 Python `QJ_TEST_PYTHON`、已有语音资源根目录 `QJ_VOICE_RESOURCES_ROOT`，可选 `QJ_TEST_PORT`。仅创建合成配置和账号标记，不复制正式数据库；真实模型只供内置样例转写。测试还会在测试子进程不可达代理下调用一键安装入口，确认已有完整资源不重复下载。

`native_cloud.mjs` 使用 `QJ_TEST_ROOT`、`QJ_TEST_EXE`、可选 `QJ_TEST_PORT`，要求 D 盘全新隔离目录（路径包含 `isolated-test`）。它验证本地原生流程与账号缓存隔离，不表示真实认证成功。

`start_live_cloud.mjs a` / `b` 启动两个隔离客户端，默认使用维护者 D 盘构建路径和测试目录，迁移环境时需调整路径。新建客户端拒绝覆盖已有数据库；`--resume` 仅恢复已有测试库；`--offline` 仅改变测试子进程代理。不要连接正式工作区或真实笔记。

真实云端测试需要预先在当前 PowerShell 会话设置以下环境变量，值不要提交到仓库：

- `QINGJIAN_TEST_SUPABASE_URL`：用户授权的免费验收项目 HTTPS 地址。
- `QINGJIAN_TEST_PUBLIC_KEY`：该项目的 Publishable / anon key，禁止管理密钥。
- `QINGJIAN_TEST_EMAIL`：可收取验证码的隔离验收邮箱。

先部署 `cloud/001_sync.sql`。`live_cloud_probe.mjs prepare 9243` / `9244` 打开登录界面；验证码只由用户在应用输入。重启后在两个隔离客户端明确选择首次关联方式，再依次运行 `live_sync_acceptance.mjs`、`live_draft_acceptance.mjs`、`live_offline_acceptance.mjs`。最后运行 `live_auth_acceptance.mjs`；此项会请求真实无效验证码直到触发限流，并退出 B，会消耗项目认证限额。

脚本会向验收账号写入合成内容，证据写入 D 盘 `isolated-test` 目录。不同真实邮箱、两台物理电脑及不同网络仍须单独验收。执行前检查所有路径、端口、账号与免费额度，不要将这些脚本用于生产数据。
