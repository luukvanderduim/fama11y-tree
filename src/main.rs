use atspi::{
    connection::set_session_accessibility,
    proxy::accessible::{AccessibleProxy, ObjectRefExt},
    zbus::{proxy::CacheProperties, Connection},
    AccessibilityConnection, Role,
};
use display_tree::{AsTree, DisplayTree, Style};
use futures::future::try_join_all;
use std::vec;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const REGISTRY_DEST: &str = "org.a11y.atspi.Registry";
const REGISTRY_PATH: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE_INTERFACE: &str = "org.a11y.atspi.Accessible";

#[derive(Debug, PartialEq, Eq, Clone)]
struct A11yNode {
    role: Role,
    children: Vec<A11yNode>,
}

impl DisplayTree for A11yNode {
    fn fmt(&self, f: &mut std::fmt::Formatter, style: Style) -> std::fmt::Result {
        self.fmt_with(f, style, &mut vec![])
    }
}

impl A11yNode {
    fn fmt_with(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        style: Style,
        prefix: &mut Vec<bool>,
    ) -> std::fmt::Result {
        for (i, is_last_at_i) in prefix.iter().enumerate() {
            // if it is the last portion of the line
            let is_last = i == prefix.len() - 1;
            match (is_last, *is_last_at_i) {
                (true, true) => write!(f, "{}", style.char_set.end_connector)?,
                (true, false) => write!(f, "{}", style.char_set.connector)?,
                // four spaces to emulate `tree`
                (false, true) => write!(f, "    ")?,
                // three spaces and vertical char
                (false, false) => write!(f, "{}   ", style.char_set.vertical)?,
            }
        }

        // two horizontal chars to mimic `tree`
        writeln!(
            f,
            "{}{} {}",
            style.char_set.horizontal, style.char_set.horizontal, self.role
        )?;

        for (i, child) in self.children.iter().enumerate() {
            prefix.push(i == self.children.len() - 1);
            child.fmt_with(f, style, prefix)?;
            prefix.pop();
        }

        Ok(())
    }
}

async fn inspect(ap: AccessibleProxy<'_>) -> Result<()> {
    println!("Inspecting object with high child count");

    let role = ap.get_role().await?;
    let description = ap.description().await?;
    let name = ap.name().await?;
    let child_count = ap.child_count().await?;
    let interfaces = ap.get_interfaces().await?;

    let first_child = ap.get_child_at_index(0).await?;
    let size_of_object = std::mem::size_of_val(&first_child);

    let size = size_of_object * child_count as usize;
    // size in human readable format
    let size = if size < 1024 {
        format!("{} B", size)
    } else if size < 1024 * 1024 {
        format!("{:.2} KB", size as f64 / 1024.0)
    } else if size < 1024 * 1024 * 1024 {
        format!("{:.2} MB", size as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", size as f64 / (1024.0 * 1024.0 * 1024.0))
    };

    let app_object = ap.get_application().await?;
    let app_ap = app_object
        .as_accessible_proxy(ap.inner().connection())
        .await?;
    let app_name = app_ap.name().await?;
    let app_role = app_ap.get_role().await?;

    println!("Object: name: \"{name}\", role: \"{role}\", description: \"{description}\"",);
    println!("Interfaces: \"{:?}\"", interfaces);
    println!(
        "child_count: {child_count}, ObjectRef size: {size_of_object}, collection size: {size},",
    );

    println!("Application: name: {app_name}, role: {app_role}");

    Ok(())
}

impl A11yNode {
    fn count_nodes_iterative(&self) -> usize {
        let mut count = 1;
        let mut stack = vec![self];

        while let Some(node) = stack.pop() {
            count += node.children.len();
            stack.extend(node.children.iter());
        }

        count
    }

    // async fn from_accessible_proxy_recursive(ap: AccessibleProxy<'_>) -> Result<A11yNode> {
    //     let connection = ap.inner().connection();
    //     if ap.child_count().await? > 100_000 {
    //         inspect(ap).await?;
    //         return Err("Child count is too high".into());
    //     }
    //     let child_objects = ap.get_children().await?;
    //     let role = ap.get_role().await?;

    //     let child_proxies = try_join_all(
    //         child_objects
    //             .iter()
    //             .map(|child| child.as_accessible_proxy(connection)),
    //     )
    //     .await?;

    //     let children = try_join_all(
    //         child_proxies
    //             .into_iter()
    //             .map(|child| Box::pin(A11yNode::from_accessible_proxy_recursive(child))),
    //     )
    //     .await?;

    //     Ok(A11yNode { role, children })
    // }

    async fn from_accessible_proxy_iterative(ap: AccessibleProxy<'_>) -> Result<A11yNode> {
        let connection = ap.inner().connection().clone();
        // Contains the processed `A11yNode`'s.
        let mut nodes: Vec<A11yNode> = Vec::new();

        // Contains the `AccessibleProxy` yet to be processed.
        let mut stack: Vec<AccessibleProxy> = vec![ap];

        // If the stack has an `AccessibleProxy`, we take the last.
        while let Some(ap) = stack.pop() {
            if ap.child_count().await? > 100_000 {
                inspect(ap).await?;
                return Err("Child count is too high".into());
            }
            let child_objects = ap.get_children().await?;
            let mut children_proxies = try_join_all(
                child_objects
                    .into_iter()
                    .map(|child| child.into_accessible_proxy(&connection)),
            )
            .await?;

            let roles = try_join_all(children_proxies.iter().map(|child| child.get_role())).await?;
            stack.append(&mut children_proxies);

            let children = roles
                .into_iter()
                .map(|role| A11yNode {
                    role,
                    children: Vec::new(),
                })
                .collect::<Vec<_>>();

            let role = ap.get_role().await?;
            nodes.push(A11yNode { role, children });
        }

        let mut fold_stack: Vec<A11yNode> = Vec::with_capacity(nodes.len());

        while let Some(mut node) = nodes.pop() {
            if node.children.is_empty() {
                fold_stack.push(node);
                continue;
            }

            // If the node has children, we fold in the children from 'fold_stack'.
            // There may be more on 'fold_stack' than the node requires.
            let begin = fold_stack.len().saturating_sub(node.children.len());
            node.children = fold_stack.split_off(begin);
            fold_stack.push(node);
        }

        fold_stack.pop().ok_or("No root node built".into())
    }
}

async fn get_registry_accessible<'a>(conn: &Connection) -> Result<AccessibleProxy<'a>> {
    let registry = AccessibleProxy::builder(conn)
        .destination(REGISTRY_DEST)?
        .path(REGISTRY_PATH)?
        .interface(ACCESSIBLE_INTERFACE)?
        .cache_properties(CacheProperties::No)
        .build()
        .await?;

    Ok(registry)
}

#[tokio::main]
async fn main() -> Result<()> {
    set_session_accessibility(true).await?;
    let a11y = AccessibilityConnection::new().await?;

    let conn = a11y.connection();
    let registry = get_registry_accessible(conn).await?;

    let no_applications = registry.child_count().await?;

    println!("Construct a tree of accessible objects on the a11y-bus (iterative method)\n");

    let now = std::time::Instant::now();
    let tree1 = A11yNode::from_accessible_proxy_iterative(registry.clone()).await?;
    let elapsed_iterative = now.elapsed();

    let node_count = tree1.count_nodes_iterative();

    println!("Construct a tree of accessible objects on the a11y-bus (recursive method)\n");

    // let now = std::time::Instant::now();
    // let tree2 = A11yNode::from_accessible_proxy_recursive(registry).await?;
    // let elapsed_recursive = now.elapsed();

    println!("| Applications | Nodes | Iterative (ms)|");
    println!("|--------------|-------|---------------|");
    println!(
        "| {:<12} | {:<5} | {:<13} |",
        no_applications,
        node_count,
        elapsed_iterative.as_millis()
    );

    // assert_eq!(tree1, tree2);
    // println!("\nBoth trees are found to be equal\n");

    println!("\nPress 'Enter' to print the tree...");
    let _ = std::io::stdin().read_line(&mut String::new());
    println!("{}", AsTree::new(&tree1));

    Ok(())
}
