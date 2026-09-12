# UART Assistant

面向 Linux 嵌入式和 MCU 开发的 PyQt5 串口调试工具。项目采用 MVC 风格组织，支持多串口会话、日志监视、VT100/ANSI 终端、XModem 传输、STM32 复位/Bootloader 控制和宏命令。

## 功能

- 多标签串口会话，可同时管理多个设备
- 监视器支持普通、HEX、ASCII + HEX 显示模式
- 时间戳、自动滚屏、关键词高亮、日志搜索和大小限制
- 断线自动重连，保留端口参数和 RX/TX 统计
- VT100/ANSI 终端模式，可直接连接目标板 Linux serial console
- XModem 文件发送与取消传输
- STM32 复位、进入 Bootloader、DTR/RTS 控制
- 宏命令编辑与快捷发送
- 基于 `QSettings` 的配置持久化

## 界面

应用顶部是串口连接区，中部是可搜索的接收监视区，下部是发送区，底部是 MCU 工具区和状态栏。终端页提供独立的 VT100/ANSI 渲染和搜索功能，适合连接嵌入式 Linux 控制台。

## 安装和运行

需要 Python 3.10 或更新版本：

```bash
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements.txt
python main.py
```

依赖：

```text
PyQt5>=5.15
pyserial>=3.5
```

Debian/Ubuntu 用户可先安装系统桌面依赖：

```bash
sudo apt install python3-venv python3-pyqt5
```

## 串口权限

大多数 Linux 发行版将 USB 串口设备放入 `dialout` 组。将当前用户加入该组后重新登录：

```bash
sudo usermod -aG dialout "$USER"
```

程序会扫描 pyserial 支持的设备，并读取 `/dev/serial/by-id/` 和 `/dev/serial/by-path/` 中的稳定别名。多设备环境下建议优先使用稳定别名，而不是依赖不断变化的 `ttyUSB0` 编号。

不要使用 root 运行本工具。DTR/RTS 的实际电平与开发板复位、BOOT0 接线有关，使用 STM32 时序前请先确认原理图。

## 快捷键

| 快捷键 | 功能 |
| --- | --- |
| `Ctrl+F` | 打开搜索 |
| `Ctrl+L` | 清空接收区 |
| `Ctrl+S` | 保存日志 |
| `Ctrl+Enter` | 发送数据 |
| `Escape` | 关闭搜索栏 |

## 项目结构

```text
uart_assistant/
├── main.py
├── model/
│   ├── serial_port.py        # pyserial 封装、参数模型、设备扫描
│   ├── terminal_emulator.py  # VT100/ANSI 终端状态机
│   ├── xmodem.py             # XModem CRC/Checksum 发送
│   └── config.py             # QSettings 配置管理
├── view/
│   ├── main_window.py        # 多标签页主窗口
│   ├── serial_widget.py      # 单个串口会话界面
│   └── dialogs.py            # 宏编辑对话框
└── controller/
    ├── serial_worker.py      # 串口线程、循环发送、MCU 控制和 XModem 调度
    └── hotplug.py            # Linux 串口设备轮询
```

## 开发验证

运行语法检查：

```bash
python -m py_compile main.py view/*.py model/*.py controller/*.py
```

在没有桌面环境的机器上可以离屏启动：

```bash
QT_QPA_PLATFORM=offscreen python main.py
```

## 贡献

提交前请保持现有业务逻辑与 UI 分层清晰，避免将通信线程和界面控件直接耦合。欢迎提交 Bug 修复、Linux 平台适配、UI 改进和协议工具扩展。

## 许可证

本项目基于 [MIT License](LICENSE) 发布。
