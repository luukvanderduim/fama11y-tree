use atspi::{
    proxy::accessible::{AccessibleProxy, ObjectRefExt},
    AccessibilityConnection, Role,
};
use display_tree::{format_tree, AsTree, DisplayTree, Style};
use zbus::proxy::CacheProperties;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const REGISTRY_DEST: &str = "org.a11y.atspi.Registry";
const REGISTRY_PATH: &str = "/org/a11y/atspi/accessible/root";
const REGISTRY_INTERFACE: &str = "org.a11y.atspi.Accessible";

#[derive(Debug)]
struct Children(Vec<A11yNode>);

impl Children {
    fn new(children: Vec<A11yNode>) -> Self {
        Children(children)
    }
}

impl DisplayTree for Children {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>, style: Style) -> std::fmt::Result {
        let idx_last = self.0.len() - 1;
        // each child but the last child with connector, last is printed with an end_connector.
        for (idx, child) in self.0.iter().enumerate() {
            let connector = if idx == idx_last {
                style.char_set.end_connector
            } else {
                style.char_set.connector
            };

            write!(
                f,
                "{}{} {}",
                connector,
                std::iter::repeat(style.char_set.horizontal)
                    .take(style.indentation as usize)
                    .collect::<String>(),
                format_tree!(*child, style)
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, DisplayTree)]
struct A11yNode {
    #[node_label]
    role: Role,

    #[tree]
    children: Children,
}

impl A11yNode {
    async fn from_accessible_proxy(ap: AccessibleProxy<'_>) -> Result<Self> {
        let role = ap.get_role().await?;
        let child_objs = ap.get_children().await?;
        let connection = ap.inner().connection();

        // Convert `Vec<ObjectRef>` to a `Vec<Future<Output = AccessibleProxy>`.
        let children = child_objs
            .iter()
            .map(|child| child.as_accessible_proxy(connection))
            .collect::<Vec<_>>();

        // Resolve the futures and filter out the errors.
        let children = futures::future::join_all(children)
            .await
            .into_iter()
            .filter_map(|child| child.ok())
            .collect::<Vec<_>>();

        // Convert to a `Vec<Future<Output = Result<A11yNode>>`.
        let children = children
            .into_iter()
            .map(|child| Box::pin(Self::from_accessible_proxy(child)))
            .collect::<Vec<_>>();

        // Resolve the futures and filter out the errors.
        let children = futures::future::join_all(children)
            .await
            .into_iter()
            .filter_map(|child| child.ok())
            .collect::<Vec<_>>();

        let children = Children::new(children);

        Ok(A11yNode { role, children })
    }
}

async fn get_registry_accessible<'a>(conn: &zbus::Connection) -> Result<AccessibleProxy<'a>> {
    let registry = AccessibleProxy::builder(conn)
        .destination(REGISTRY_DEST)?
        .path(REGISTRY_PATH)?
        .interface(REGISTRY_INTERFACE)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;

    Ok(registry)
}

#[tokio::main]
async fn main() -> Result<()> {
    // set_session_accessibility(true).await?;
    let a11y = AccessibilityConnection::new().await?;

    let conn = a11y.connection();
    let registry = get_registry_accessible(conn).await?;

    let no_children = registry.child_count().await?;
    println!("Number of accessible applications on the a11y-bus: {no_children}");
    println!("Construct a tree of accessible objects on the a11y-bus\n");

    let now = std::time::Instant::now();
    let tree = A11yNode::from_accessible_proxy(registry).await?;
    let elapsed = now.elapsed();
    println!("Elapsed time: {:?}", elapsed);

    println!("\nPress 'Enter' to print the tree...");
    let _ = std::io::stdin().read_line(&mut String::new());

    println!("{}", AsTree::new(&tree));

    Ok(())
}
