use std::collections::HashMap;

use graphviz_rust::dot_structures::*;
use rand::{thread_rng, Rng};
use rerun::{
    components::{Arrow3D, ColorRGBA, Label, Point3D, Radius},
    MsgSender,
};

const MAX_DIMS: usize = 10;

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

trait Lerpable {
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl Lerpable for u8 {
    fn lerp(self, other: u8, t: f32) -> u8 {
        (self as f32 * (1.0 - t) + other as f32 * t).round() as u8
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
    let mut labels = Vec::with_capacity(node_indices.len());
    {
        let mut color_map = HashMap::new();
        for node in nodes.values() {
            let mut color = ColorRGBA::from_rgb(
                rng.gen_range(0..255),
                rng.gen_range(0..255),
                rng.gen_range(0..255),
            );
            let mut label = Label(node.id.0.to_string());
            for a in &node.attributes {
                let a0 = a.0.to_string();
                let a1 = a.1.to_string();
                match a0.as_str() {
                    "label" => {
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
                        label = Label(s[start..end].to_string());
                    }
                    "shape" => match color_map.get(&a1) {
                        Some(&c) => {
                            color = c;
                        }
                        None => {
                            color_map.insert(a1, color);
                        }
                    },
                    _ => (),
                }
            }
            colors.push(color);
            labels.push(label);
        }
        println!("color_map:\n{:?}", color_map);
    }
    let colors = colors;
    let labels = labels;
    assert_eq!(colors.len(), num_points);
    assert_eq!(labels.len(), num_points);

    let edge_strength = 0.1;
    let edge_length = 1.0;
    let node_repelling_strength = 0.1;
    let node_repelling_distance = 2.0;
    let float_strength = 0.02;
    let float_distance = 2.0;

    // Init points with random values in many dimensions
    let mut points = vec![[0.0; MAX_DIMS]; num_points];
    for i in 0..num_points {
        for v in &mut points[i] {
            *v = rng.gen_range(-1.0..1.0);
        }
    }

    // Gradually reduce the number of dimensions while solving the constraints
    for dims in (3..MAX_DIMS).rev() {
        for _ in 0..10 {
            for _ in 0..10 {
                // Move parents upwards and children downwards
                for &(i, j) in &edges_indices {
                    let p1 = &points[i];
                    let p2 = &points[j];
                    let dz = p1[2] - p2[2];
                    if dz < float_distance {
                        points[i][2] += float_strength;
                        points[j][2] -= float_strength;
                    }
                }

                // Move nodes away from each other
                for i in 0..num_points {
                    for j in i + 1..num_points {
                        let length = points_distance(&points, i, j, dims);
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
                    let length = points_distance(&points, i, j, dims);
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
                MsgSender::new(format!("nodes/{}", &labels[i].0))
                    .with_component(&[point])?
                    .with_component(&[colors[i].clone()])?
                    .with_component(&[labels[i].clone()])?
                    .with_splat(Radius(0.05))?
                    .send(&session)?;
            }

            let mut arrows = Vec::with_capacity(edges_indices.len());
            let mut arrow_colors = Vec::with_capacity(edges_indices.len());
            for &(i, j) in &edges_indices {
                let length = points_distance(&points, i, j, dims);
                let p1 = &points[i];
                let p2 = &points[j];
                arrows.push(Arrow3D {
                    origin: [p1[0], p1[1], p1[2]].into(),
                    vector: [p2[0] - p1[0], p2[1] - p1[1], p2[2] - p1[2]].into(),
                });
                arrow_colors.push(if length < edge_length {
                    let t = ((edge_length - length) / 0.5).clamp(0.0, 1.0);
                    ColorRGBA::from_rgb(0.lerp(255, t), 255.lerp(0, t), 0)
                } else {
                    let t = ((length - edge_length) / 5.0).clamp(0.0, 1.0);
                    ColorRGBA::from_rgb(0.lerp(127, t), 255.lerp(0, t), 0.lerp(255, t))
                });
            }
            assert_eq!(arrows.len(), edges_indices.len());
            assert_eq!(arrow_colors.len(), edges_indices.len());
            MsgSender::new("edges")
                .with_component(&arrows)?
                .with_component(&arrow_colors)?
                .send(&session)?;
        }
    }

    // rerun::native_viewer::show(&session)?;

    Ok(())
}

fn points_distance(points: &[[f32; MAX_DIMS]], i: usize, j: usize, dims: usize) -> f32 {
    let p1 = &points[i];
    let p2 = &points[j];
    let mut length_squared: f32 = 0.0;
    for k in 0..dims {
        let dk = p2[k] - p1[k];
        length_squared += dk * dk;
    }
    length_squared.sqrt()
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
