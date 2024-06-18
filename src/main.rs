use atspi::{
    connection::set_session_accessibility,
    proxy::{
        accessible::{AccessibleProxy, ObjectRefExt},
        component::ComponentProxy,
    },
    zbus::{proxy::CacheProperties, Connection},
    AccessibilityConnection, Interface, Role,
};
use display_tree::{DisplayTree, Style};
use futures::future::try_join_all;
use std::vec;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const REGISTRY_DEST: &str = "org.a11y.atspi.Registry";
const REGISTRY_PATH: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE_INTERFACE: &str = "org.a11y.atspi.Accessible";
const COMPONENT_INTERFACE: &str = "org.a11y.atspi.Component";

#[derive(Debug, PartialEq, Eq, Clone)]
struct A11yNode {
    role: Role,
    zorder: i16,
    children: Vec<A11yNode>,
}

impl PartialOrd for A11yNode {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.zorder.cmp(&other.zorder))
    }
}

impl Ord for A11yNode {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.zorder.cmp(&other.zorder)
    }
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

impl A11yNode {
    fn as_vec(&self) -> Vec<&A11yNode> {
        let mut nodes = vec![self];
        let mut stack = vec![self];

        while let Some(node) = stack.pop() {
            stack.extend(node.children.iter());
            nodes.extend(node.children.iter());
        }

        nodes
    }

    async fn from_accessible_proxy_iterative(ap: AccessibleProxy<'_>) -> Result<A11yNode> {
        let connection = ap.inner().connection().clone();

        // Contains the processed `A11yNode`'s.
        let mut nodes: Vec<A11yNode> = Vec::new();
        // Contains the `AccessibleProxy` yet to be processed.
        let mut stack: Vec<AccessibleProxy> = vec![ap];

        let black_list = ["org.a11y.atspi.Registry", ":1.0"];

        // If the stack has an `AccessibleProxy`, we take the last.
        while let Some(ap) = stack.pop() {
            let mut has_component = ap.get_interfaces().await?.contains(Interface::Component);

            let bus_name = ap.inner().destination().as_str();
            if black_list.contains(&bus_name) {
                has_component = false;
            }

            let child_objects = ap.get_children().await?;
            let mut children_proxies = try_join_all(
                child_objects
                    .iter()
                    .cloned()
                    .map(|child| child.into_accessible_proxy(&connection)),
            )
            .await?;

            let roles = try_join_all(children_proxies.iter().map(|child| child.get_role())).await?;

            if !has_component {
                let children = roles
                    .into_iter()
                    .map(|role| A11yNode {
                        role,
                        zorder: -1,
                        children: Vec::new(),
                    })
                    .collect();

                let role = ap.get_role().await?;

                nodes.push(A11yNode {
                    role,
                    zorder: -1,
                    children,
                });

                stack.append(&mut children_proxies);
                continue;
            }

            let component_proxies = try_join_all(child_objects.into_iter().map(|child| {
                ComponentProxy::builder(&connection)
                    .destination(child.name)
                    .unwrap()
                    .path(child.path)
                    .unwrap()
                    .interface(COMPONENT_INTERFACE)
                    .unwrap()
                    .cache_properties(CacheProperties::No)
                    .build()
            }))
            .await?;

            let orders =
                try_join_all(component_proxies.iter().map(|child| child.get_mdiz_order())).await?;

            let roles_n_orders = roles.into_iter().zip(orders.into_iter());

            stack.append(&mut children_proxies);

            let children = roles_n_orders
                .map(|(role, zorder)| A11yNode {
                    role,
                    zorder,
                    children: Vec::new(),
                })
                .collect();

            let role = ap.get_role().await?;

            let ap_object: atspi::ObjectRef = ap.try_into()?;

            let component_proxy = ComponentProxy::builder(&connection)
                .destination(ap_object.name)?
                .path(ap_object.path)?
                .interface(COMPONENT_INTERFACE)?
                .cache_properties(CacheProperties::No)
                .build()
                .await?;

            let zorder = component_proxy.get_mdiz_order().await?;

            nodes.push(A11yNode {
                role,
                zorder,
                children,
            });
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
    let tree1 = A11yNode::from_accessible_proxy_iterative(registry.clone()).await?;

    let mut node_vec = tree1.as_vec();
    node_vec.sort_unstable();

    println!("Displaying the top 25 zorder nodes in the tree");
    for (idx, node) in node_vec.iter().rev().enumerate() {
        println!("{}: {}, zorder: {}", idx, node.role, node.zorder);
        if idx > 25 {
            break;
        }
    }

    Ok(())
}
