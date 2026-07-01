# OSAL 后端行为契约

## 1. 目的

本文档定义了每个 OSAL 后端必须满足的精确行为契约。它面向三类读者：

- **API 设计者** (Phase 2)：从下方契约中推导出 trait 签名。
- **后端实现者**：精确了解每个方法必须做什么。
- **测试编写者**：从前置/后置条件和错误表格中直接派生测试用例。

契约描述了**什么是正确行为**，而非**后端如何实现**。只要可观察行为一致，两个后端可以使用完全不同的内部机制。

---

## 2. 运行时模型

OSAL 公共 API 面向 `no_std` 环境设计。

对于动态对象模型（boxed callback、动态队列、任务参数、运行时注册表），
OSAL 默认要求运行环境提供 allocator。allocator 支持是**项目级的运行时前提**，
而非可选的 Cargo feature。每个需要堆分配的 crate 无条件声明
`extern crate alloc`。

Rust `std` 支持仅在可用时启用可选的宿主机集成功能（如
`impl std::error::Error`）。它**不**决定后端是否可用。POSIX 和 FreeRTOS
后端保持 `no_std`，通过 FFI 或平台 crate（`libc`、RTOS C API）调用原生系统能力。

`std` Cargo feature 保留用于未来的仅宿主机功能。构建、测试或使用任何后端
均不需要它。

---

## 3. 非目标

以下内容明确**不属于**本契约的范围：

- 性能特性（延迟、吞吐量）
- 实时性保证（截止时间调度、优先级反转防护）
- 内存布局或分配策略
- 超出所列方法的调试或内省功能
- 同一进程内不同后端之间的互操作性
- 从 ISR 上下文滥用的安全性（调用者负责调用正确的变体）

后端可以提供超出本契约的额外能力，但可移植的应用代码不得依赖它们。

---

## 4. 后端选择模型

OSAL 应用在编译时链接**恰好一个**后端。后端通过 `osal` 门面 crate 的 Cargo feature 标志选择：

```toml
[dependencies]
osal = { version = "0.1" }                       # POSIX（默认）
osal = { version = "0.1", features = ["mock"] }   # Mock
```

只能启用一个后端 feature。尝试启用多个后端会导致编译错误。

所有公共类型（Mutex、Queue、Task 等）解析为活动后端的具体实现。应用代码绝不应直接导入后端 crate。

---

## 5. 通用错误语义

所有可能失败的操作返回 `Result<T, osal_api::Error>`。

### 错误变体

| 变体 | 含义 | 典型原因 |
|------|------|----------|
| `OutOfMemory` | 分配失败 | 堆耗尽 |
| `Timeout` | 操作超时 | `Timeout::After(d)` 到期 |
| `QueueFull` | 队列容量已满 | 非阻塞发送到满队列 |
| `QueueEmpty` | 队列中无消息 | 非阻塞接收自空队列 |
| `LockFailed` | 无法获取锁 | 互斥锁被其他上下文持有 |
| `NotFound` | 资源未找到 | 无效的 handle 或 ID |
| `InvalidParameter` | 参数超出合法范围 | 零长度名称、count > max |
| `AlreadyInitialized` | 资源已创建/启动 | 对 Task 调用两次 `spawn()` |
| `NotInitialized` | 资源尚未启动 | 对未启动的 Task 执行 `join()` |
| `Unsupported` | 后端无法执行此操作 | FreeRTOS 上 ISR 互斥锁 |
| `Internal(&'static str)` | 意外的原生错误 | errno、FreeRTOS 状态码 |

### 规则

1. 本契约中列出的每个错误路径必须生成所指定的确切变体——后端不得替换为不同的变体。
2. `Internal` 变体是最后手段，用于没有明显映射的平台错误。必须携带标识来源的静态字符串（如 `"pthread_mutex_lock: EINVAL"`）。
3. `Timeout` 仅在时间限定的等待到期且未成功时返回。不会为立即失败（如队列满）返回此错误。
4. `Error` 类型不携带生命周期参数，不包含堆分配数据（除 `Internal(&'static str)` 外）。

---

## 6. 时间与超时语义

### 主要类型

```rust
use core::time::Duration;

pub enum Timeout {
    NoWait,            // 立即返回，绝不阻塞
    After(Duration),   // 最多阻塞指定的时长
    Forever,           // 无限期阻塞
}
```

`core::time::Duration` 在 `no_std` 中可用，作为 OSAL API 中的通用时间表示。

### 超时行为

| 超时 | 行为 |
|------|------|
| `NoWait` | 调用必须立即返回。如果操作本会阻塞，返回适当的错误（`QueueFull`、`QueueEmpty`、`LockFailed` 或 `Timeout`）。 |
| `After(d)` | 阻塞直到成功或 `d` 已过，取先到者。如果 `d` 到期，返回 `Error::Timeout`。调用不得在 `d` 未过时提前返回 `Timeout`（不得有虚假唤醒）。由于调度原因可能稍晚返回。 |
| `Forever` | 阻塞直到成功或致命错误。不得返回 `Error::Timeout`。 |

### 时钟契约

- `Clock::now()` 返回自任意起点（通常是进程启动或系统启动）起单调递增的 `Duration`。
- 时钟绝不倒退。
- 精度取决于后端；可移植代码不得假设亚毫秒精度。
- `Clock::elapsed(since: Duration) -> Duration` 等价于 `now() - since`，在零处饱和。

### 延迟契约

- `Clock::delay(d: Duration)` 将调用任务阻塞**至少** `d` 时间。可能因调度而更长。
- `delay(Duration::ZERO)` 必须立即返回。
- 实现应使用可用的最高效阻塞原语（如 `nanosleep`、`pthread_cond_timedwait`、RTOS tick delay）。

---

## 7. 对象生命周期

所有 OSAL 对象遵循通用生命周期：

```
Create ──→ (Start) ──→ Use ──→ Delete / Drop
```

### 创建

- 构造函数接受配置参数（容量、消息大小、最大计数、栈大小、优先级等）。
- 无效参数（零容量、count > max）返回 `Error::InvalidParameter`。
- 分配失败返回 `Error::OutOfMemory`。

### 使用

- 对象创建后立即可用（除非文档中指定了显式的 `start` 步骤）。
- 对已删除/已 drop 对象的操作具有未定义行为。后端应尽力安全地失败，但不提供保证。
- 除文档另有说明外，所有公共方法是线程安全的。

### 删除

- `Drop`（或如果需要异步清理，则使用显式的 `delete`/`close` 方法）释放所有资源。
- 在已删除对象上阻塞的任务必须被唤醒并收到错误。
- 删除后，对象的 handle（如果有）变为无效。

---

## 8. Task 契约

### 类型：`Task`

一个独立的执行上下文（线程 / RTOS 任务）。

### 创建

```rust
pub struct TaskBuilder { ... }

impl TaskBuilder {
    pub fn new() -> Self;
    pub fn name(self, name: &str) -> Self;
    pub fn stack_size(self, bytes: usize) -> Self;
    pub fn priority(self, prio: Priority) -> Self;
    pub fn spawn<F>(self, entry: F) -> Result<Task>
        where F: FnOnce() + Send + 'static;
}
```

### Builder 规则

| 字段 | 默认值 | 合法范围 |
|------|--------|----------|
| `name` | `""` (空) | 0..31 字节，不含内嵌 NUL |
| `stack_size` | `4096` | 最小为后端定义值（通常 512） |
| `priority` | `1` | 0..(后端最大值 - 1) |

- 如果任何字段超出范围，`spawn` 返回 `Error::InvalidParameter`。
- 如果无法分配任务，`spawn` 返回 `Error::OutOfMemory`。
- 入口函数 `F` 在新任务中精确执行一次。
- `spawn` 返回 `Ok` 后，任务处于 `Ready` 状态。
- 不能对同一 builder 调用两次 `spawn`（它会消耗 `self`）。

### 生命周期方法

```rust
impl Task {
    pub fn join(self, timeout: Timeout) -> Result<ExitCode>;
    pub fn handle(&self) -> Handle;
    pub fn priority(&self) -> Priority;
}
```

- `join(timeout)`：阻塞直到任务退出。
  - 成功 join 后返回 `Ok(ExitCode)`。
  - 如果任务未在超时时间内退出，返回 `Error::Timeout`。
  - 如果任务从未启动，返回 `Error::NotInitialized`。
  - `join` 返回 `Ok` 后，任务 handle 无效且 `Task` 被消耗。
- `handle()`：返回唯一标识此任务的透明 `Handle`。
- `priority()`：返回任务的当前优先级。

### 静态方法

```rust
impl Task {
    pub fn current() -> Handle;
    pub fn count() -> usize;
}
```

- `current()`：返回调用者任务的 handle。必须可从任何 OSAL 任务上下文工作。
- `count()`：返回系统当前已知的任务数。包括正在运行、就绪、阻塞和挂起的任务。

### 任务状态

| 状态 | 含义 |
|------|------|
| `Ready` | 任务已创建，有资格运行 |
| `Running` | 任务正在执行 |
| `Blocked` | 任务在等待同步原语 |
| `Suspended` | 任务被显式挂起（依赖后端） |
| `Finished` | 任务入口函数已返回 |

- 状态转换是后端依赖的。可移植代码仅将状态查询用于诊断目的——不得使用状态来做正确性决策。

### 退出码

```rust
pub struct ExitCode(u32);

impl ExitCode {
    pub const SUCCESS: ExitCode = ExitCode(0);
    pub fn new(code: u32) -> Self;
    pub fn code(&self) -> u32;
}
```

---

## 9. Mutex 契约

### 类型：`Mutex<T>`

保护类型 `T` 的值的可重入互斥锁。

### 创建

```rust
impl<T> Mutex<T> {
    pub fn new(value: T) -> Result<Self>;
}
```

- 分配互斥锁并存储 `value`。
- 分配失败返回 `Error::OutOfMemory`。

### 锁定

```rust
impl<T> Mutex<T> {
    pub fn lock(&self, timeout: Timeout) -> Result<MutexGuard<T>>;
    pub fn isr_lock(&self) -> Result<MutexGuard<T>>;
}
```

- `lock(timeout)`：
  - 获取互斥锁，最多阻塞 `timeout` 时间。
  - 成功时返回 `MutexGuard`，通过 `DerefMut` 提供 `&mut T` 访问。
  - 丢弃 `MutexGuard` 释放一层锁。
  - 可重入：拥有者任务可以再次调用 `lock` 而不阻塞。每次 `lock` 必须匹配一次对应的 guard drop。
  - 超时到期返回 `Error::Timeout`。
  - 如果是 `Timeout::NoWait` 且互斥锁被其他任务持有，返回 `Error::LockFailed`。
- `isr_lock()`：
  - 非阻塞：立即返回。
  - 在没有真正 ISR 上下文的平台（POSIX、Mock）上，等同于 `lock(Timeout::NoWait)`。
  - 在不支持 ISR 互斥锁操作的平台上返回 `Error::Unsupported`。

### MutexGuard

```rust
pub struct MutexGuard<'a, T> { ... }

impl<T> Deref for MutexGuard<'_, T> { type Target = T; ... }
impl<T> DerefMut for MutexGuard<'_, T> { ... }
impl<T> Drop for MutexGuard<'_, T> { /* 释放一层锁 */ }
```

- `MutexGuard` 是 `!Send`（它代表任务局部锁的所有权）。
- 互斥锁已被删除时丢弃 guard 具有未定义行为（guard 不应活得比互斥锁更长）。

### 删除

- 在锁定时丢弃 `Mutex<T>`：行为由后端定义。在 POSIX 上，互斥锁被销毁；在 FreeRTOS 上，这是未定义的。可移植代码必须确保互斥锁在 drop 前已解锁。

---

## 10. Semaphore 契约

### 类型：`CountingSemaphore`

用于资源管理和任务信号传递的计数信号量。

### 创建

```rust
impl CountingSemaphore {
    pub fn new(max_count: u32, initial_count: u32) -> Result<Self>;
    pub fn max_count(&self) -> u32;
    pub fn count(&self) -> u32;
}
```

- `new(max, initial)`：
  - 如果 `initial > max` 或 `max == 0`，返回 `Error::InvalidParameter`。
  - 分配失败返回 `Error::OutOfMemory`。
- `max_count()`：返回配置的最大计数。
- `count()`：返回当前计数（快照；返回后可能立即改变）。

### 操作

```rust
impl CountingSemaphore {
    pub fn acquire(&self, timeout: Timeout) -> Result<()>;
    pub fn release(&self) -> Result<()>;

    pub fn isr_acquire(&self) -> Result<()>;
    pub fn isr_release(&self) -> Result<()>;
}
```

- `acquire(timeout)`：
  - 如果 `count > 0`：减 1 并返回 `Ok(())`。
  - 如果 `count == 0` 且 `NoWait`：返回 `Error::Timeout`。
  - 如果 `count == 0` 且 `After(d)`：阻塞直到 `release()` 唤醒我们或超时到期。
  - 如果 `count == 0` 且 `Forever`：阻塞直到 `release()` 唤醒我们。
  - 每次 `release()` 精确唤醒一个被阻塞的获取者。
- `release()`：
  - 如果 `count < max_count`：加 1 并唤醒一个获取者。
  - 如果 `count == max_count`：返回 `Error::InvalidParameter`（信号量已满）。
- `isr_acquire()`：非阻塞；等同于 `acquire(Timeout::NoWait)`。
- `isr_release()`：ISR 安全；可从中断上下文调用。

### 类型：`BinarySemaphore`

`max_count = 1` 的 `CountingSemaphore` 便利包装。

```rust
impl BinarySemaphore {
    pub fn new() -> Result<Self>;
    pub fn acquire(&self, timeout: Timeout) -> Result<()>;
    pub fn release(&self) -> Result<()>;
    pub fn is_acquired(&self) -> bool;
    pub fn isr_acquire(&self) -> Result<()>;
    pub fn isr_release(&self) -> Result<()>;
}
```

- `new()`：以 `count = 0`、`max_count = 1` 创建。
- `is_acquired()`：如果 `count == 1` 返回 `true`。
- 所有其他方法委托给底层的 `CountingSemaphore`。

---

## 11. Queue 契约

### 类型：`Queue`

用于任务间字节消息通信的有界 FIFO 消息队列。

### 创建

```rust
impl Queue {
    pub fn new(capacity: usize, msg_size: usize) -> Result<Self>;
    pub fn capacity(&self) -> usize;
    pub fn msg_size(&self) -> usize;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_full(&self) -> bool;
}
```

- `new(capacity, msg_size)`：
  - 如果 `capacity == 0` 或 `msg_size == 0`，返回 `Error::InvalidParameter`。
  - 分配失败返回 `Error::OutOfMemory`。
- `capacity()`：最大消息数。
- `msg_size()`：每条消息的固定大小（字节）。
- `len()`：当前队列中的消息数。
- `is_empty()` / `is_full()`：便捷查询。

### 操作

```rust
impl Queue {
    pub fn send(&self, data: &[u8], timeout: Timeout) -> Result<()>;
    pub fn recv(&self, buffer: &mut [u8], timeout: Timeout) -> Result<()>;
    pub fn close(&self);

    pub fn isr_send(&self, data: &[u8]) -> Result<()>;
    pub fn isr_recv(&self, buffer: &mut [u8]) -> Result<()>;
}
```

- `send(data, timeout)`：
  - `data.len()` 必须等于 `msg_size()`；否则 `Error::InvalidMessageSize`。
  - 如果未满：将 `data` 复制到队列中，唤醒一个被阻塞的接收者，返回 `Ok(())`。
  - 如果已满且 `NoWait`：返回 `Error::QueueFull`。
  - 如果已满且 `After(d)`：阻塞直到有空间或超时。
  - 如果已满且 `Forever`：阻塞直到有空间可用。
  - 如果队列已被 `close()`: 返回 `Error::QueueClosed`。
- `recv(buffer, timeout)`：
  - `buffer.len()` 必须等于 `msg_size()`；否则 `Error::InvalidMessageSize`。
  - 如果非空：将最旧消息复制到 `buffer` 中，唤醒一个被阻塞的发送者，返回 `Ok(())`。
  - 如果为空且 `NoWait`：返回 `Error::QueueEmpty`。
  - 如果为空且 `After(d)`：阻塞直到有消息或超时。
  - 如果为空且 `Forever`：阻塞直到有消息。
  - 如果队列已被 `close()` 且为空：返回 `Error::QueueClosed`。
- `close()`：
  - 标记队列为已关闭。
  - 唤醒所有被阻塞的发送者和接收者（它们返回 `Error::QueueClosed`）。
  - 后续操作返回 `Error::QueueClosed`。
  - 幂等：多次调用 `close()` 是安全的。
- `isr_send(data)`：非阻塞；等同于 `send(data, Timeout::NoWait)`。
- `isr_recv(buffer)`：非阻塞；等同于 `recv(buffer, Timeout::NoWait)`。

### FIFO 保证

消息按发送顺序接收。如果任务 A 依次发送 M1、M2，然后任务 B 接收两次，则 B 收到 M1 然后是 M2。

---

## 12. Timer 契约

### 类型：`Timer`

在指定周期后调用回调的软件定时器。

### 创建

```rust
pub enum TimerMode {
    OneShot,
    Periodic,
}

pub type TimerCallback = Box<dyn Fn() + Send + 'static>;

impl Timer {
    pub fn new(
        name: &str,
        period: Duration,
        mode: TimerMode,
        callback: TimerCallback,
    ) -> Result<Self>;
}
```

- `new(name, period, mode, callback)`：
  - 如果 `period` 为零，返回 `Error::InvalidParameter`。
  - 分配失败返回 `Error::OutOfMemory`。
  - 定时器在**已停止**状态下创建。
  - 回调在定时器到期时被调用。它不得 panic（panicking 的回调在 `panic=abort` 下会中止进程）。

### 操作

```rust
impl Timer {
    pub fn start(&self) -> Result<()>;
    pub fn stop(&self) -> Result<()>;
    pub fn reset(&self) -> Result<()>;
    pub fn change_period(&self, new_period: Duration) -> Result<()>;
}
```

- `start()`：
  - 开始倒计时。如果已在运行，行为等同于 `reset()`。
  - 回调在 `period` 过去后触发。
- `stop()`：
  - 阻止未来的回调。正在执行的回调不会被中断。
  - 如果已经停止，这是空操作。
- `reset()`：
  - 从现在重新开始倒计时。如果已停止，同时启动定时器。
- `change_period(new_period)`：
  - 更新周期。在下次到期时生效。
  - 如果 `new_period` 为零，返回 `Error::InvalidParameter`。

### 回调执行

- **OneShot**：回调触发一次，然后定时器停止。
- **Periodic**：回调触发后，定时器自动重载。下次倒计时从预定到期时间开始（而非从回调完成时间），在实际可行的情况下。
- 回调在定时器管理锁之外执行。
- 回调在定时器服务上下文中执行（而非 ISR）。
- 回调应该是短小且非阻塞的。

### 删除

- 丢弃 `Timer` 会停止它并释放资源。正在执行的回调不会被中断。

---

## 13. 不支持的能力规则

某些后端无法实现所有操作。以下规则规定了如何处理不支持的能力：

1. **返回 `Error::Unsupported`。** 操作必须始终返回此错误，而非其他错误或 panic。
2. **记录文档。** 每个后端的模块级文档必须列出所有返回 `Error::Unsupported` 的能力。
3. **契约测试必须跳过。** 符合性测试框架提供了一种机制，为声明某能力不支持的后端跳过测试。
4. **禁止静默成功。** 后端不得对实际未执行的操作返回 `Ok`。有信号但无唤醒是不可接受的。

### 已知后端限制

| 能力 | POSIX | Mock | 未来的 FreeRTOS |
|------|-------|------|-----------------|
| ISR 操作 | Try-lock，非阻塞 | Try-lock，非阻塞 | 真正的 ISR |
| 任务优先级 | 信息性的 | 确定性顺序 | 硬件优先级 |
| 栈高水位 | 不跟踪 | 不跟踪 | 硬件跟踪 |
| 调度器 start/stop | 空操作 | 可控 | 硬件调度器 |
| 临界区 | 递归互斥锁 | 递归互斥锁 | 中断禁用 |
| 任务 suspend/resume | 不支持 | 支持 | 支持 |

---

## 14. Mock 后端要求

Mock 后端（`osal-backend-mock`）是一个完全在内存中的、确定性的实现，用于单元测试和契约验证。

### 必要能力

1. **确定性时间**：一个只有在显式指示时才前进的假时钟。不经过真实时间。
2. **故障注入**：每个操作都有可配置的故障触发器，导致指定的错误。例如：
   - "下一次 acquire 以 Timeout 失败"
   - "第 3 次 send 以 QueueFull 失败"
3. **操作记录**：每次调用（方法、参数、返回值）都记录在历史日志中。测试通过此日志进行断言。
4. **默认所有操作非阻塞**：通过时间推进模拟阻塞。`Timeout::Forever` 阻塞直到相应的唤醒事件发生。
5. **确定性任务顺序**：任务按优先级顺序运行；同等优先级下，先启动的先运行。上下文切换发生在显式让出点。

### 契约测试集成

Mock 后端是契约测试的主要目标。本文档中的每个行为要求都必须能够针对 Mock 后端进行测试。在 Mock 和 POSIX 上都通过的测试被认为是已验证的。

---

## 15. POSIX 后端要求

POSIX 后端（`osal-backend-posix`）使用 pthread 和相关 POSIX API 实现所有 OSAL 原语。

### 必要的原语

| OSAL 类型 | POSIX 实现 |
|-----------|-----------|
| Mutex | `pthread_mutex_t`（PTHREAD_MUTEX_RECURSIVE 用于 raw，PTHREAD_MUTEX_ERRORCHECK 用于 `Mutex<T>`） |
| CountingSemaphore | `pthread_mutex_t` + `pthread_cond_t` + count 变量 |
| Queue | `pthread_mutex_t` + 两个 `pthread_cond_t`（not_empty、not_full）+ 环形缓冲区 |
| Task | `pthread_create` / `pthread_join` |
| Timer | 后台工作线程中带 CLOCK_MONOTONIC 的 `pthread_cond_timedwait` |
| Clock | `clock_gettime(CLOCK_MONOTONIC)` |
| Critical section | 进程级递归 `pthread_mutex_t`，通过 `pthread_key_t` TLS 记录每线程嵌套深度 |

### 具体要求

1. **单调时钟**：所有时间操作使用 `CLOCK_MONOTONIC`。挂钟变化不得影响 OSAL 时序。
2. **线程安全初始化**：全局状态（时钟 epoch、注册表）使用 `pthread_once_t`。
3. **无真正的 ISR**：`isr_*` 方法是非阻塞 try-操作。它们绝不得阻塞。
4. **调度器空操作**：`System::start()` 和 `System::stop()` 是已记录的的空操作。任务在创建时即运行。
5. **优先级是信息性的**：任务优先级仅在显式启用实时调度时才映射到 pthread 调度策略属性。
6. **堆报告**：`heap_free()` 返回 `usize::MAX`（主机虚拟内存）。
7. **协作式取消**：任务删除请求取消；任务必须定期检查并退出。
8. **线程注册表**：注册表跟踪所有 OSAL 任务以进行内省（`count()`、`current()`）。

---

## 16. 符合性测试矩阵

每个行为要求映射到一个或多个契约测试。后端必须通过所有非跳过测试。

### 图例

- **R**: Required — 所有后端必须通过
- **P**: POSIX only — 需要主机操作系统特性
- **M**: Mock only — 测试故障注入或确定性行为
- **S**: Skipped — 后端声明此能力不支持

### Task 测试

| 测试 | 要求 | POSIX | Mock |
|------|------|-------|------|
| 以默认配置创建 | Builder 默认值编译并通过 spawn | R | R |
| 以全部字段创建 | name、stack、priority 正确传播 | R | R |
| 拒绝零长度名称 | `Error::InvalidParameter` | R | R |
| 拒绝零栈大小 | `Error::InvalidParameter` | R | R |
| spawn 并成功 join | 任务运行，join 返回 ExitCode | R | R |
| 带超时的 join | 未退出的任务返回 `Error::Timeout` | R | R |
| join 未启动的任务 | `Error::NotInitialized` | R | R |
| 多个并发任务 | 3+ 个任务同时运行 | R | R |
| 任务内调用 current() | 返回正确的 handle | R | R |
| count() 反映实际情况 | 与已启动的任务数匹配 | R | R |
| suspend / resume | 任务暂停和恢复 | S | R |

### Mutex 测试

| 测试 | 要求 | POSIX | Mock |
|------|------|-------|------|
| 创建并存储值 | `Mutex::new(v)` 正常工作 | R | R |
| 锁定和开锁 | Guard 提供 &mut T，drop 释放 | R | R |
| 可重入锁定 | 同一任务锁 N 次，解锁 N 次 | R | R |
| 跨任务互斥 | 锁持有时其他任务阻塞 | R | R |
| 非阻塞 try-lock | 被持有时 `Timeout::NoWait` 返回 `LockFailed` | R | R |
| 超时到期 | `Timeout::After(d)` 返回 `Timeout` | R | R |
| Forever 阻塞直到释放 | `Timeout::Forever` 在释放后成功 | R | R |
| Guard 是 `!Send` | 编译时检查 | R | R |
| isr_lock 非阻塞 | 立即返回 | R | R |

### Semaphore 测试

| 测试 | 要求 | POSIX | Mock |
|------|------|-------|------|
| 以有效计数创建 | `new(max, initial)` 正常工作 | R | R |
| 拒绝 initial > max | `Error::InvalidParameter` | R | R |
| 拒绝 max == 0 | `Error::InvalidParameter` | R | R |
| acquire 递减 count | count 从 N 变为 N-1 | R | R |
| release 递增 count | count 从 N 变为 N+1 | R | R |
| 为空时 acquire 阻塞 | 任务等待直到 release | R | R |
| 为空时超时 | `Timeout::After(d)` 返回 `Timeout` | R | R |
| 满时 release 失败 | `Error::InvalidParameter` | R | R |
| release 精确唤醒一个 | N 次 release 唤醒 N 个等待者，不多 | R | R |
| BinarySemaphore 基础 | `new()`、`acquire()`、`release()` | R | R |
| isr_acquire 非阻塞 | 立即返回 | R | R |
| 从 ISR 上下文 isr_release | Mock 模拟 ISR 上下文 | R | R |

### Queue 测试

| 测试 | 要求 | POSIX | Mock |
|------|------|-------|------|
| 以有效参数创建 | `Queue::new(cap, size)` 正常工作 | R | R |
| 拒绝零容量 | `Error::InvalidParameter` | R | R |
| 拒绝零 msg_size | `Error::InvalidParameter` | R | R |
| 发送并接收单条消息 | 往返保留字节 | R | R |
| FIFO 顺序 | 消息按发送顺序接收 | R | R |
| 满时 send 阻塞 | 发送者等待直到 recv | R | R |
| 空时 recv 阻塞 | 接收者等待直到 send | R | R |
| 满时非阻塞 send | `Error::QueueFull` | R | R |
| 空时非阻塞 recv | `Error::QueueEmpty` | R | R |
| 消息大小不匹配 | send/recv 时 `Error::InvalidMessageSize` | R | R |
| close 唤醒被阻塞的发送者 | 未决 send 返回 `QueueClosed` | R | R |
| close 唤醒被阻塞的接收者 | 未决 recv 返回 `QueueClosed` | R | R |
| close 是幂等的 | 调用两次 close 是安全的 | R | R |
| close 后操作 | 全部返回 `QueueClosed` | R | R |

### Timer 测试

| 测试 | 要求 | POSIX | Mock |
|------|------|-------|------|
| 创建单次定时器 | `Timer::new(... OneShot)` 成功 | R | R |
| 创建周期定时器 | `Timer::new(... Periodic)` 成功 | R | R |
| 拒绝零周期 | `Error::InvalidParameter` | R | R |
| 单次触发一次 | 回调被精确调用一次 | R | R |
| 周期触发多次 | 回调被调用 >= 2 次 | R | R |
| stop 阻止回调 | 停止的定时器不触发 | R | R |
| reset 重新开始倒计时 | 定时器在 reset 后 period 时触发 | R | R |
| change period 更新时序 | 新周期生效 | R | R |
| 回调在锁外执行 | 回调中嵌套定时器操作 OK | R | R |

### Clock 和 System 测试

| 测试 | 要求 | POSIX | Mock |
|------|------|-------|------|
| now() 单调递增 | `now()` 绝不递减 | R | R |
| elapsed() 正确 | `elapsed(s) + s ≈ now()` | R | R |
| delay() 阻塞至少 d | delay 后 tick count 增加 | R | R |
| delay(0) 立即返回 | 零延迟接近瞬时 | R | R |
| heap_free() 返回值 | POSIX 上非零，usize::MAX 可接受 | R | R |
| task_count() 返回任务数 | 与已启动的任务数匹配 | R | R |
| 临界区互斥 | 嵌套 enter/exit 是安全的 | R | R |

---

---

> 关于对象所有权、生命周期、克隆语义和销毁安全性规则，
> 请参阅 [Object Lifetime and Ownership](object-lifetime-zh.md)。
