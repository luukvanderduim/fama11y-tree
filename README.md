# fama11y-tree

An example on how to build a tree of all objects using [atspi](https://github.com/odilia-app/atspi).

```Text
     Root
     ├── Child1
     │   ├── Grandchild1
     │   └── Grandchild2
     │       ├── Great-Grandchild1
     │       └── Great-Grandchild2
     └── Child2
         ├── Grandchild1
         └── Grandchild2
```

## MSRV

### Async fn recursive version

The `A11yNode::from_accessible_proxy_recursive` method currently requires Rust 1.77.2 or higher.

Cargo-msrv reports:

```sh
error[E0733]: recursion in an `async fn` requires boxing                                                                                                                             │
│   --> src/main.rs:67:5                                                                                                                                                               │
│    |                                                                                                                                                                                 │
│ 67 |     async fn from_accessible_proxy(ap: AccessibleProxy<'_>) -> Result<A11yNode> {                                                                                               │
│    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ recursive `async fn`                                                                            │
│    |                                                                                                                                                                                 │
│    = note: a recursive `async fn` must be rewritten to return a boxed `dyn Future`                                                                                                   │
│    = note: consider using the `async_recursion` crate: https://crates.io/crates/async_recursion
```

### Async iterative version

The `A11yNode::from_accessible_proxy_iterative` method requires Rust 1.75.0 or higher.

## Performance

| processes | objects | recursive method | iterative method |
|:----------------------:|:-----------------:|:----------------:|:----------------:|
| 25  | 241  | 11.87871ms  | 31.792694ms  |
| 26  | 2900  | 288.716071ms  | 474.438153ms  |
| 28  | 4960  | 992.864247ms  | 808.82431ms  |
| 27  | 6948  | 1.447283253s  | 1.077571422s  |
| 28  | 7509  | 1.515564922s  | 1.08042886s  |
| 30  | 7813  | 1.589614858s  | 1.119880292s  |

It appears 'recursive' is faster with a small number of accessible applications,
however 'iterative´ is faster when a larger number of objects is exposed on the bus.
Note that it does not take that many applications to reach the point where iterative is faster.

## Known issues

### xdg-dbus-proxy

`xdg-dbus-proxy` does not seem to implement all methods on the `Accessible` interface.
This results in an error if applcations require it:

```sh
Error: MethodError(OwnedErrorName("org.freedesktop.DBus.Error.UnknownMethod"), Some("Method \"GetRole\" with signature \"\" on interface \"org.a11y.atspi.Accessible\" doesn't exist\n"), Msg { type: Error, serial: 53, sender: UniqueName(":1.106"), reply-serial: 48, body: Signature("s"), fds: [] })
```

### LibreOffice Calc

It appears LibreOffice Calc exposes 2^31 accessible objects (for the table cells), which leads to an ~~interesting~~ impractical and probably impossible situation.

If fama11y-tree calls `GetChildren` on the `Accessible` interface of their parent frame, it will try and send them all.

We can gather ~7000 objects/s (as observed above), 2^31 objects would take about 3 days and 14 hours.
Aside from the practical downside (we can't cache the table on behalf of screen-readers), it is also illegal
to send a D-Bus message larger than 128 megabytes.

Needless to say that this is not the user experience AT-SPI2 users would like.

In practical sense, one would like only those cells exposed which are visible.

[Related bug: 156657](https://bugs.documentfoundation.org/show_bug.cgi?id=156657)
