use std::collections::HashMap;

use graphviz_rust::dot_structures::*;
use rand::{thread_rng, Rng};
use rerun::{
    components::{Arrow3D, ColorRGBA, Label, Point3D, Radius},
    MsgSender,
};

pub fn id_to_string(id: &Id) -> String {
    match id {
        Id::Html(ref v) => format!("html {}", v),
        Id::Escaped(ref v) => format!("esc {}", v),
        Id::Plain(ref v) => format!("plain {}", v),
        Id::Anonymous(ref v) => format!("anon {}", v),
    }
}

pub fn port_to_string(port: &Port) -> String {
    match port {
        Port(None, None) => "".to_string(),
        Port(Some(ref id), None) => id_to_string(id),
        Port(None, Some(ref dir)) => format!(":{}", dir),
        Port(Some(ref id), Some(ref dir)) => format!("{}:{}", id_to_string(id), dir),
    }
}

pub fn node_id_to_string(node_id: &NodeId) -> String {
    match node_id.1 {
        None => id_to_string(&node_id.0),
        Some(ref port) => format!("{}:{}", id_to_string(&node_id.0), port_to_string(port)),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = thread_rng();

    let dot = std::fs::read_to_string("../Reconstruction/docs/graphviz/cmake/gg.dot")?;
    let g: Graph = graphviz_rust::parse(dot.as_str())?;
    let statements = match g {
        Graph::Graph {
            id: _,
            strict: _,
            ref stmts,
        }
        | Graph::DiGraph {
            id: _,
            strict: _,
            ref stmts,
        } => stmts,
    };

    let session = rerun::SessionBuilder::new("my_app").connect(rerun::default_server_addr());

    let nodes: HashMap<String, Node> = statements
        .iter()
        .filter_map(|s| match s {
            Stmt::Node(n) => Some((node_id_to_string(&n.id), n.clone())),
            _ => None,
        })
        .collect();

    let mut node_indices = HashMap::<String, usize>::new();
    for key in nodes.keys() {
        node_indices.insert(key.clone(), node_indices.len());
    }
    let node_indices = node_indices;
    let num_points = node_indices.len();

    let mut edges = Vec::<(NodeId, NodeId)>::new();
    for s in statements {
        match s {
            Stmt::Edge(Edge {
                ty: EdgeTy::Pair(Vertex::N(a), Vertex::N(b)),
                attributes: _,
            }) => edges.push((a.clone(), b.clone())),
            Stmt::Edge(Edge {
                ty: EdgeTy::Chain(chain),
                attributes: _,
            }) => {
                for i in 0..chain.len() - 1 {
                    if let (Vertex::N(a), Vertex::N(b)) = (&chain[i], &chain[i + 1]) {
                        edges.push((a.clone(), b.clone()))
                    }
                }
            }
            _ => (),
        }
    }
    let edges = edges;

    let edges_indices = edges
        .iter()
        .map(|(a, b)| {
            (
                *node_indices.get(&node_id_to_string(a)).unwrap(),
                *node_indices.get(&node_id_to_string(b)).unwrap(),
            )
        })
        .collect::<Vec<_>>();

    let mut colors = Vec::with_capacity(node_indices.len());
    for _ in 0..num_points {
        colors.push(ColorRGBA::from_rgb(
            rng.gen_range(0..255),
            rng.gen_range(0..255),
            rng.gen_range(0..255),
        ));
    }
    let colors = colors;
    let labels = nodes
        .values()
        .map(|node| {
            for a in &node.attributes {
                if a.0.to_string() == "label" {
                    let s = match &a.1 {
                        Id::Html(s) => s,
                        Id::Escaped(s) => s,
                        Id::Plain(s) => s,
                        Id::Anonymous(s) => s,
                    };
                    let start = match s.rfind('/') {
                        Some(i) => i + 1,
                        None => match s.find('"') {
                            Some(i) => i + 1,
                            None => 0,
                        },
                    };
                    let end = s.rfind('"').unwrap_or(s.len());
                    return Label(s[start..end].to_string());
                }
            }
            Label(node.id.0.to_string())
        })
        .collect::<Vec<_>>();

    let edge_strength = 0.1;
    let edge_length = 1.0;
    let node_repelling_strength = 0.1;
    let node_repelling_distance = 2.0;
    let float_strength = 0.02;
    let float_distance = 2.0;

    // Init points with random values in many dimensions
    const MAX_DIMS: usize = 10;
    let mut points = vec![[0.0; MAX_DIMS]; num_points];
    for i in 0..num_points {
        for v in &mut points[i] {
            *v = rng.gen_range(-1.0..1.0);
        }
    }

    // Gradually reduce the number of dimensions while solving the constraints
    for dims in (3..MAX_DIMS).rev() {
        for _ in 0..100 {
            // Move parents upwards and children downwards
            for &(i, j) in &edges_indices {
                let p1 = &points[i];
                let p2 = &points[j];
                let dz = p2[2] - p1[2];
                if dz < float_distance {
                    points[i][2] -= float_strength;
                    points[j][2] += float_strength;
                }
            }

            // Move nodes away from each other
            for i in 0..num_points {
                for j in i + 1..num_points {
                    let p1 = &points[i];
                    let p2 = &points[j];
                    let length = {
                        let mut length_squared: f32 = 0.0;
                        for k in 0..dims {
                            let dk = p2[k] - p1[k];
                            length_squared += dk * dk;
                        }
                        length_squared.sqrt()
                    };

                    if length < node_repelling_distance {
                        let c = node_repelling_distance - length;
                        let d = c.min(node_repelling_strength) * 0.5 / length.max(0.001);
                        for k in 0..dims {
                            let u = (points[j][k] - points[i][k]) * d;
                            points[i][k] -= u;
                            points[j][k] += u;
                        }
                    }
                }
            }

            // Move nodes to satisfy edge length
            for &(i, j) in &edges_indices {
                let p1 = &points[i];
                let p2 = &points[j];
                let length = {
                    let mut length_squared: f32 = 0.0;
                    for k in 0..dims {
                        let dk = p2[k] - p1[k];
                        length_squared += dk * dk;
                    }
                    length_squared.sqrt()
                };

                if length < node_repelling_distance {
                    let c = length - edge_length;
                    let d = edge_strength * c * -0.5 / length.max(0.001);
                    for k in 0..dims {
                        let u = (points[j][k] - points[i][k]) * d;
                        points[i][k] -= u;
                        points[j][k] += u;
                    }
                }
            }

            for i in 0..num_points {
                let point = Point3D {
                    x: points[i][0],
                    y: points[i][1],
                    z: points[i][2],
                };
                MsgSender::new(format!("nodes/{i}"))
                    .with_component(&[point])?
                    .with_component(&[colors[i].clone()])?
                    .with_component(&[labels[i].clone()])?
                    .with_splat(Radius(0.05))?
                    .send(&session)?;
            }

            let arrows = edges
                .iter()
                .map(|(a, b)| {
                    let a = node_indices.get(&node_id_to_string(a)).unwrap();
                    let b = node_indices.get(&node_id_to_string(b)).unwrap();
                    let a = &points[*a];
                    let b = &points[*b];
                    Arrow3D {
                        origin: [a[0], a[1], a[2]].into(),
                        vector: [b[0] - a[0], b[1] - a[1], b[2] - a[2]].into(),
                    }
                })
                .collect::<Vec<_>>();
            MsgSender::new("edges")
                .with_component(&arrows)?
                .send(&session)?;
        }
    }

    // rerun::native_viewer::show(&session)?;

    Ok(())
}

#[cfg(test)]
mod test {

    use graphviz_rust::dot_generator::*;
    use graphviz_rust::dot_structures::*;
    use graphviz_rust::parse;

    #[test]
    fn parse_test() {
        let g: Graph = parse(
            r#"
        strict digraph t {
            aa[color=green]
            subgraph v {
                aa[shape=square]
                subgraph vv{a2 -> b2}
                aaa[color=red]
                aaa -> bbb
            }
            aa -> be -> subgraph v { d -> aaa}
            aa -> aaa -> v
        }
        "#,
        )
        .unwrap();

        assert_eq!(
            g,
            graph!(strict di id!("t");
            node!("aa";attr!("color","green")),
            subgraph!("v";
            node!("aa"; attr!("shape","square")),
            subgraph!("vv"; edge!(node_id!("a2") => node_id!("b2"))),
            node!("aaa";attr!("color","red")),
            edge!(node_id!("aaa") => node_id!("bbb"))
                ),
                edge!(node_id!("aa") => node_id!("be") => subgraph!("v"; edge!(node_id!("d") => node_id!("aaa")))),
                edge!(node_id!("aa") => node_id!("aaa") => node_id!("v"))
            )
        )
    }
}
