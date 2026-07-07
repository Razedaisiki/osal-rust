# OSAL 架构设计文档

> **注意：** 本文档为辅助中文说明。MVP 快速迭代期间，英文
> [`docs/architecture.md`](architecture.md) 和代码 rustdoc
> 为权威来源。若中英文存在不一致，以英文文档为准。

## 1. 概述

OSAL（Operating System Abstraction Layer，操作系统抽象层）是一个分层式的 Rust 框架，用于构建可移植的嵌入式和实时应用。它允许你编写一次应用逻辑，通过切换 Cargo feature flag 即可在不同平台（POSIX 主机、实时内核、Mock 环境）上运行。

## 2. 设计目标

- **可移植应用**：应用代码只依赖 `osal` 门面 crate。切换平台只需修改一个 Cargo feature，无需重写代码。
- **后端独立**：各类后端实现通过公共 trait 相互隔离。添加新后端不需要修改应用代码或其他后端。
- **契约驱动质量**：每个后端必须通过相同的行为契约测试，确保跨平台语义一致。
- **清晰分层**：每层只依赖其下层。平台细节绝不泄漏到公共 API。

## 3. 分层架构

```
应用层
    ↓
osal（门面 crate）
    ↓
osal-api（公共 trait 和类型）
    ↓
+----------+     +-----------------+
| osal-    |     | osal-portable   |
| shared   |     |（可移植辅助工具）   |
+----------+     +-----------------+
    ↓                   ↓
+---------------------------+
| osal-backend-*            |  ← 平台特定实现
|（posix, freertos, mock）   |
+---------------------------+
    ↓
osal-bsp + osal-bsp-*   ← 板级支持包
    ↓
原生 OS / RTOS / 硬件
```

### 3.1 `osal-api` — 基础层

基础 crate。定义 OSAL **能做什么**，而非**怎么做**。

- 所有 OS 原语的公共 trait（Mutex, Semaphore, Queue, Task, Timer, Clock, EventFlags, System）
- 共享类型：`Error`、`Timeout`、`Result<T>`、`Handle`、`Priority`、`EventMask`、`StackSize`
- 零运行时依赖
- 默认 `no_std` 兼容；可选的 `std` feature

后端 crate 实现这些 trait。`osal` 门面重导出用户所需的一切。

### 3.2 `osal-shared` — OS 无关共享逻辑

所有后端共用的实现：

- 通用参数校验辅助（`validate_queue_capacity`、`validate_send_message_size` 等）
- 关闭状态跟踪（`CloseFlag`）
- 初始化与生命周期状态管理

全局对象 ID 注册表和对象表由
[ADR 0006](adr/0006-object-handle-model.md) 推迟。MVP 使用强类型句柄
（`Queue`、`Mutex<T>`、`Timer`），通过后端自适应的所有权模型
（`Arc`、`Rc`、原生句柄）而非中心化数字 ID 注册表。

没有此 crate，每个后端都会重新发明校验和生命周期逻辑，导致不一致。

### 3.3 `osal-portable` — 可复用辅助工具

多个后端可选用的实用工具：

- 环形缓冲区实现
- 时间转换辅助函数
- `no_std` 静态内存池
- 不支持功能的 fallback 空操作实现

这些是**内部构建模块**，不属于公共 API。

### 3.4 `osal-backend-*` — 平台实现

每个后端 crate 为特定平台实现所有 `osal-api` trait：

| Crate | 平台 | 用途 |
|-------|------|------|
| `osal-backend-posix` | Linux, macOS, POSIX | 开发、CI、模拟 |
| `osal-backend-mock` | 进程内 fake | 单元测试、契约验证 |
| `osal-backend-freertos` | FreeRTOS | ARM Cortex-M, RISC-V 嵌入式 |

后端依赖 `osal-api`、`osal-shared`，可选依赖 `osal-portable`。后端之间不能互相依赖。

### 3.5 `osal-bsp` + `osal-bsp-*` — 板级支持

将平台硬件配置与 OS 后端逻辑分离：

- 启动与引导钩子
- 控制台/调试输出
- 时钟和定时器硬件抽象
- 中断控制器配置
- 内存和堆区域设置
- 资源限制（最大任务数、最大队列数等）

BSP crate 位于 OSAL 层之下，独立于后端选择。

### 3.6 `osal-testkit` — 测试基础设施

共享测试工具：

- 契约测试框架：对任意后端执行相同行为测试
- 断言辅助：OSAL 特定验证的通用模式
- 模拟时钟：确定性的可重现时序
- 故障注入框架

### 3.7 `osal` — 门面

用户唯一需要依赖的 crate：

```toml
[dependencies]
osal = "0.1"
```

职责：
- 重导出 `osal-api` 类型
- 通过门面 Cargo features 选择后端（`backend-posix`、`backend-mock`、未来的 `backend-freertos`）
- 编译时防止多后端同时启用
- 提供 `prelude` 模块方便导入

## 4. 依赖关系图

```
osal-api  ←── osal-shared ←── osal-portable ←── osal-backend-posix
    ↑              ↑
    +── osal-bsp ←── osal-bsp-linux
    +── osal-testkit
    +── osal-backend-mock
    +── osal（门面）
```

无循环依赖。每个 crate 只依赖其下层的 crate。

## 5. Feature 标志

### 5.1 门面级 features

```toml
[features]
default = ["backend-posix"]
backend-posix = ["dep:osal-backend-posix"]
backend-mock = ["dep:osal-backend-mock"]
```

规则：
- 编译时必须有且仅有一个后端被启用
- `backend-posix` 作为默认值方便开发
- `backend-mock` 用于测试

### 5.2 环境级 features

```toml
std = ["osal-api/std", "osal-shared/std"]
alloc = ["osal-api/alloc", "osal-shared/alloc"]
```

- `std`：启用标准库（用于主机测试运行器、示例）
- `alloc`：启用堆分配但不依赖完整 `std`

## 6. 命名约定

| 方面 | 约定 | 示例 |
|------|------|------|
| Crate 命名 | `osal-{层次}` | `osal-api`, `osal-backend-posix` |
| Trait 命名 | 直接使用名词 | `pub trait Mutex`, `pub trait Task` |
| 模块文件名 | `snake_case.rs` | `event_flags.rs`, `clock.rs` |
| 错误类型 | `Error`（无生命周期参数） | `Error::Timeout` |
| 布尔语义返回 | `Result<(), Error>` | `fn lock(&self) -> Result<()>` |
| ISR 方法 | `isr_` 前缀 | `isr_lock()`, `isr_signal()` |
| 后端类型 | 描述性名称 | `Priority`, `EventMask`, `StackSize` |
| Prelude 导入 | `use osal::prelude::*` | |
| 时间类型 | `core::time::Duration` + `Timeout` 枚举 | `Timeout::After(d)` |

## 7. 错误处理策略

OSAL 使用 `osal-api` 中单一的平铺式 `Error` 枚举：

```rust
pub enum Error {
    OutOfMemory,   // 内存不足
    Timeout,       // 操作超时
    QueueFull,     // 队列已满
    QueueEmpty,    // 队列为空
    LockFailed,    // 获取锁失败
    NotFound,      // 资源未找到
    InvalidParameter, // 无效参数
    Unsupported,   // 该后端不支持此操作
    Internal(&'static str), // 内部错误
    // ...
}

pub type Result<T> = core::result::Result<T, Error>;
```

**无生命周期参数** — 保持类型 `Send + Sync + 'static`。

**布尔语义操作**（lock, signal, wait）返回 `Result<(), Error>` 而非自定义布尔类型。这更符合 Rust 惯用法，并与 `?` 操作符良好集成。

**后端错误**（errno、FreeRTOS 状态码）在后端实现内部映射为 OSAL 错误。原始平台错误码绝不出现在公共 API 中。

## 8. 模块组织模式

每个 crate 内部模块遵循以下模式：

```
crates/osal-api/src/
├── lib.rs          # crate 根，模块声明
├── error.rs        # Error 枚举和 Result 别名
├── time.rs         # Timeout、duration 辅助函数
├── types.rs        # 通用类型别名
├── traits.rs       # trait 模块声明
├── traits/
│   ├── mutex.rs    # 互斥锁
│   ├── semaphore.rs # 信号量
│   ├── queue.rs    # 队列
│   ├── task.rs     # 任务
│   ├── timer.rs    # 定时器
│   ├── clock.rs    # 时钟
│   ├── event_flags.rs # 事件标志
│   └── system.rs   # 系统控制
└── prelude.rs      # 选择性重导出
```

后端 crate 镜像 trait 结构，提供具体实现：

```
crates/osal-backend-posix/src/
├── lib.rs
├── task.rs
├── mutex.rs
├── semaphore.rs
├── queue.rs
├── timer.rs
├── clock.rs
└── sys/            # 薄 FFI 包装
    ├── pthread.rs
    ├── condvar.rs
    ├── clock.rs
    └── errno.rs
```

## 9. 添加新后端

添加新后端的步骤：

1. 创建 `crates/osal-backend-{name}/`，`Cargo.toml` 依赖 `osal-api` + `osal-shared`
2. 实现所有 `osal-api` trait
3. 在 `crates/osal/Cargo.toml` 中添加 feature 标志
4. 通过 `osal-testkit` 的契约测试套件

无需修改 `osal-api`、`osal-shared` 或现有后端。
