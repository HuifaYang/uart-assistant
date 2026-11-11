# UART Assistant

面向 Windows / Linux 嵌入式与 MCU 开发的轻量串口调试工具。使用 Rust + Slint 编写，release 构建为单个约 8 MB 的可执行文件，不依赖 Python 解释器、Qt 或 .NET 运行时。

## 功能

- 多标签串口会话：新建、关闭、切换、双击重命名，每个标签独立保存参数与统计数据
- 设备发现：Windows 枚举 COM 口并显示 VID/PID、产品名；Linux 优先使用 `/dev/serial/by-id`、`/dev/serial/by-path` 稳定别名；支持手动输入任意设备路径
- 串口参数：波特率、数据位、停止位、校验位、流控
- 监视器：普通 / HEX / ASCII+HEX 三种显示模式，时间戳、自动滚屏、暂停、编码切换（UTF-8 / GB18030 / GBK / Big5 / Shift_JIS / Latin-1）、行数上限、日志搜索、关键词高亮、清空与保存日志
- 后台原始 RX 记录：在显示限流之前写入原始字节，每 10 MiB 自动分卷；暂停、清空、切换显示模式均不影响记录
- 断线自动重连：1.5 s 起指数退避，最长 30 s；热插拔检测到设备后立即探测，重连保留 RX/TX 统计
- VT100/ANSI 终端：CSI/OSC 解析、SGR 颜色、光标控制、备用屏幕与回滚缓冲，可作为目标板 Linux 串口控制台
- XModem 128 字节 / 1K 文件发送：CRC 或 Checksum 协商、超时重试、取消传输与进度显示
- STM32 复位 / 进入 Bootloader 时序，以及 DTR / RTS 线路控制
- 发送区：循环发送、HEX 发送、自动加换行、TX 回显、多行输入、宏命令快捷按钮
- 配置持久化：上次使用的端口与参数、每设备独立档案（最多 40 条）、宏命令与监视器偏好

## 构建与运行

需要 Rust（建议 1.85 或更新）以及目标平台的系统依赖：

- Windows：Visual Studio Build Tools（MSVC 工具链）+ Windows SDK
- Debian/Ubuntu：`sudo apt install libudev-dev libgl1 libxkbcommon-x11-0 libfontconfig1`

```bash
cargo build --release
```

生成的可执行文件：

```text
target/release/uart-assistant        # Linux
target/release/uart-assistant.exe    # Windows
```

`Cargo.lock` 已提交，可用 `cargo build --release --locked` 复现依赖。

本仓库在 Windows 上附带一份安装在 `.toolchain/` 内的 Rust 工具链（不写系统目录、不修改 PATH，且已被 git 忽略）。使用这份本地工具链时先设置环境变量：

```powershell
$env:RUSTUP_HOME = "$PWD\.toolchain\rustup"
$env:CARGO_HOME  = "$PWD\.toolchain\cargo"
$env:Path        = "$PWD\.toolchain\cargo\bin;$env:Path"
cargo build --release
```

Linux 上访问串口通常需要把用户加入 `dialout` 组后重新登录；不要以 root 运行本工具。DTR/RTS 的实际电平与开发板复位、BOOT0 接线有关，使用 STM32 时序前请确认原理图。

## 配置

配置文件由所在平台决定：

```text
Windows  %APPDATA%\uart-assistant\config.json
Linux    ~/.config/uart-assistant/config.json
```

## 项目结构

```text
uart_assistant/
├── Cargo.toml / Cargo.lock / build.rs
├── src/
│   ├── main.rs / app.rs        # 入口、Slint 桥接、事件泵
│   ├── port.rs / session.rs    # 跨平台设备枚举、串口线程、重连与热插拔
│   ├── monitor.rs / terminal.rs # 监视器格式化与 VT100/ANSI 状态机
│   ├── raw_log.rs / xmodem.rs  # 原始分卷记录与 XModem 协议
│   └── config.rs / types.rs    # 配置持久化与共享类型
├── ui/
│   ├── app.slint               # 主窗口、宏编辑对话框
│   └── theme.slint             # 配色与可复用控件
└── .github/workflows/ci.yml    # Windows / Ubuntu 双平台构建
```

## 开发验证

```bash
cargo check
cargo test
cargo build --release
```

CI 在 `windows-latest` 与 `ubuntu-latest` 上执行 `cargo check`、`cargo test` 与 `cargo build --release`，并上传两个平台的可执行文件作为构建产物。

## 当前实现边界

- 终端以等宽网格和前景色渲染，ANSI 背景色用前景色/反色近似；未提供鼠标右键复制粘贴与全局 Ctrl 快捷键
- 发送历史保存在内存中，暂未提供下拉选择；窗口几何尚未持久化
- 尚无自动化回归测试，真实串口收发、XModem 与文件对话框需在实际硬件与交互场景下验证

## 许可

项目代码按 [MIT License](LICENSE) 发布。

第三方依赖中，Slint 采用 GPLv3 / 商业双许可：个人使用或全代码开源无额外限制；若计划闭源分发，请先确认其许可条款。
