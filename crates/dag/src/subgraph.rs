// SPDX-FileCopyrightText: Copyright © 2020-2025 Serpent OS Developers
//
// SPDX-License-Identifier: MPL-2.0

use petgraph::{EdgeType, prelude::Graph, stable_graph::IndexType, visit::Dfs};

/// Given an input [`Graph`] and the start nodes, construct a subgraph
/// Used largely in transposed form for reverse dependency calculation
pub fn subgraph<N, E, Ty, Ix>(graph: &Graph<N, E, Ty, Ix>, starting_nodes: &[N]) -> Graph<N, E, Ty, Ix>
where
    N: PartialEq + Clone,
    E: Clone,
    Ix: IndexType,
    Ty: EdgeType,
{
    let add_node = |graph: &mut Graph<N, E, Ty, Ix>, node| {
        if let Some(index) = graph.node_indices().find(|i| graph[*i] == node) {
            index
        } else {
            graph.add_node(node)
        }
    };

    let mut res = Graph::default();
    let mut dfs = Dfs::empty(&graph);

    for starting_node in starting_nodes {
        let Some(starting_node_index) = graph.node_indices().find(|n| graph[*n] == *starting_node) else {
            continue;
        };

        dfs.move_to(starting_node_index);

        while let Some(node) = dfs.next(&graph) {
            let node_index = add_node(&mut res, graph[node].clone());
            for neighbor in graph.neighbors_directed(node, petgraph::Direction::Outgoing) {
                if let Some(edge) = graph.find_edge(node, neighbor) {
                    let neighbor_index = add_node(&mut res, graph[neighbor].clone());
                    res.add_edge(node_index, neighbor_index, graph[edge].clone());
                }
            }
        }
    }

    res
}

#[cfg(test)]
mod test {
    use petgraph::{
        data::{Element, FromElements},
        prelude::DiGraph,
        visit::{Reversed, Topo, Walker},
    };

    use super::*;

    #[test]
    fn basic_topo() {
        let graph: DiGraph<i32, ()> = DiGraph::from_elements([
            Element::Node { weight: 1 },
            Element::Node { weight: 2 },
            Element::Node { weight: 3 },
            Element::Node { weight: 4 },
            Element::Edge {
                source: 0,
                target: 1,
                weight: (),
            },
            Element::Edge {
                source: 0,
                target: 2,
                weight: (),
            },
            Element::Edge {
                source: 1,
                target: 2,
                weight: (),
            },
            Element::Edge {
                source: 2,
                target: 3,
                weight: (),
            },
        ]);
        let subg = subgraph(&graph, &[2]);
        let topo = Topo::new(&subg);
        let order: Vec<i32> = topo.iter(&subg).map(|n| subg[n]).collect();

        assert_eq!(order, vec![2, 3, 4]);
    }

    #[test]
    fn reverse_topo() {
        let graph: DiGraph<i32, ()> = DiGraph::from_elements([
            Element::Node { weight: 1 },
            Element::Node { weight: 2 },
            Element::Node { weight: 3 },
            Element::Node { weight: 4 },
            Element::Edge {
                source: 0,
                target: 1,
                weight: (),
            },
            Element::Edge {
                source: 0,
                target: 2,
                weight: (),
            },
            Element::Edge {
                source: 1,
                target: 2,
                weight: (),
            },
            Element::Edge {
                source: 2,
                target: 3,
                weight: (),
            },
        ]);
        let subg = subgraph(&graph, &[2]);
        let revg = Reversed(&subg);
        let removal: Vec<i32> = Topo::new(revg).iter(revg).map(|n| subg[n]).collect();
        assert_eq!(removal, vec![4, 3, 2]);
    }

    // TODO: break cycles!
    #[ignore = "cycles breaking needs to be implemented"]
    #[test]
    fn cyclic_topo() {
        let graph: DiGraph<i32, ()> = DiGraph::from_elements([
            Element::Node { weight: 1 },
            Element::Node { weight: 2 },
            Element::Node { weight: 3 },
            Element::Node { weight: 4 },
            Element::Node { weight: 5 },
            Element::Edge {
                source: 0,
                target: 1,
                weight: (),
            },
            Element::Edge {
                source: 0,
                target: 2,
                weight: (),
            },
            Element::Edge {
                source: 1,
                target: 3,
                weight: (),
            },
            Element::Edge {
                source: 1,
                target: 4,
                weight: (),
            },
            Element::Edge {
                source: 2,
                target: 4,
                weight: (),
            },
            Element::Edge {
                source: 3,
                target: 0,
                weight: (),
            },
        ]);
        let subg = subgraph(&graph, &[1, 4]);
        let revg = Reversed(&subg);
        let removal: Vec<i32> = Topo::new(revg).iter(revg).map(|n| subg[n]).collect();
        assert_eq!(removal, vec![5, 3, 4, 2, 1]);
    }
}
