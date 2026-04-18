use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::algo::toposort;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use crate::definition::{DagDefinition, NodeDef};

/// A compiled DAG backed by petgraph, ready for execution.
#[derive(Debug)]
pub struct DagGraph {
    graph: DiGraph<NodeDef, Option<String>>,
    node_map: HashMap<String, NodeIndex>,
}

impl DagGraph {
    /// Build a petgraph from a DAG definition.
    pub fn build(def: &DagDefinition) -> anyhow::Result<Self> {
        let mut graph = DiGraph::new();
        let mut node_map = HashMap::new();

        for node in &def.nodes {
            let idx = graph.add_node(node.clone());
            node_map.insert(node.id.clone(), idx);
        }

        for edge in &def.edges {
            let from = node_map[&edge.from];
            let to = node_map[&edge.to];
            graph.add_edge(from, to, edge.label.clone());
        }

        let dag = Self { graph, node_map };
        dag.validate_acyclic()?;
        Ok(dag)
    }

    fn validate_acyclic(&self) -> anyhow::Result<()> {
        toposort(&self.graph, None).map_err(|e| {
            let node = &self.graph[e.node_id()];
            anyhow::anyhow!("cycle detected at node: {}", node.id)
        })?;
        Ok(())
    }

    /// Topological order of node IDs.
    pub fn topological_order(&self) -> anyhow::Result<Vec<String>> {
        let sorted = toposort(&self.graph, None).map_err(|e| {
            let node = &self.graph[e.node_id()];
            anyhow::anyhow!("cycle detected at node: {}", node.id)
        })?;
        Ok(sorted
            .iter()
            .map(|idx| self.graph[*idx].id.clone())
            .collect())
    }

    /// Group nodes into execution levels for parallel execution.
    /// Nodes at the same level have no dependencies between them.
    pub fn execution_levels(&self) -> anyhow::Result<Vec<Vec<String>>> {
        let topo = toposort(&self.graph, None)
            .map_err(|e| anyhow::anyhow!("cycle: {}", self.graph[e.node_id()].id))?;

        let mut level_of: HashMap<NodeIndex, usize> = HashMap::new();

        for &idx in &topo {
            let max_parent_level = self
                .graph
                .neighbors_directed(idx, Direction::Incoming)
                .map(|parent| level_of.get(&parent).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);

            let my_level = if self
                .graph
                .neighbors_directed(idx, Direction::Incoming)
                .next()
                .is_none()
            {
                0
            } else {
                max_parent_level + 1
            };

            level_of.insert(idx, my_level);
        }

        let max_level = level_of.values().copied().max().unwrap_or(0);
        let mut levels: Vec<Vec<String>> = vec![Vec::new(); max_level + 1];

        for (idx, level) in &level_of {
            levels[*level].push(self.graph[*idx].id.clone());
        }

        // Remove empty levels
        levels.retain(|l| !l.is_empty());
        Ok(levels)
    }

    pub fn get_node(&self, id: &str) -> Option<&NodeDef> {
        self.node_map.get(id).map(|idx| &self.graph[*idx])
    }

    /// Incoming edges to `id`: `(predecessor_node_id, edge_label)`.
    pub fn incoming_edges(&self, id: &str) -> Vec<(String, Option<String>)> {
        let Some(&idx) = self.node_map.get(id) else {
            return Vec::new();
        };
        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|e| (self.graph[e.source()].id.clone(), e.weight().clone()))
            .collect()
    }

    /// Topological order of nodes in the **loop body** of `loop_id`.
    ///
    /// Body roots are outgoing neighbors connected with edge label `"body"`. A node is in the body
    /// when it is reachable from those roots and every predecessor is either `loop_id` or already
    /// in the body set (so merge points fed from outside the loop are excluded).
    pub fn loop_body_topological_order(&self, loop_id: &str) -> anyhow::Result<Vec<String>> {
        let Some(&loop_ix) = self.node_map.get(loop_id) else {
            anyhow::bail!("unknown loop node id: {loop_id}");
        };

        let body_roots: Vec<NodeIndex> = self
            .graph
            .edges(loop_ix)
            .filter(|e| e.weight().as_deref() == Some("body"))
            .map(|e| e.target())
            .collect();

        if body_roots.is_empty() {
            anyhow::bail!(
                "loop node '{loop_id}' must have at least one outgoing edge with label \"body\""
            );
        }

        let mut body_ix: HashSet<NodeIndex> = HashSet::new();
        let mut q: VecDeque<NodeIndex> = VecDeque::new();
        for r in body_roots {
            body_ix.insert(r);
            q.push_back(r);
        }

        while let Some(u) = q.pop_front() {
            for e in self.graph.edges(u) {
                let v = e.target();
                let preds_ok = self
                    .graph
                    .neighbors_directed(v, Direction::Incoming)
                    .all(|p| p == loop_ix || body_ix.contains(&p));
                if preds_ok && body_ix.insert(v) {
                    q.push_back(v);
                }
            }
        }

        let topo = self.topological_order()?;
        Ok(topo
            .into_iter()
            .filter(|id| {
                self.node_map
                    .get(id)
                    .map(|ix| body_ix.contains(ix))
                    .unwrap_or(false)
            })
            .collect())
    }

    /// Get nodes downstream of a given node, optionally filtered by edge label.
    pub fn successors(&self, id: &str, label_filter: Option<&str>) -> Vec<String> {
        let Some(&idx) = self.node_map.get(id) else {
            return Vec::new();
        };
        self.graph
            .edges(idx)
            .filter(|e| match label_filter {
                Some(label) => e.weight().as_deref() == Some(label),
                None => true,
            })
            .map(|e| self.graph[e.target()].id.clone())
            .collect()
    }

    /// Get all root nodes (no incoming edges).
    pub fn roots(&self) -> Vec<String> {
        self.node_map
            .iter()
            .filter(|(_, idx)| {
                self.graph
                    .neighbors_directed(**idx, Direction::Incoming)
                    .next()
                    .is_none()
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }
}
