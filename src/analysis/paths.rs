//! Parcours : les suites de pages les plus fréquentes, niveau par niveau, sous la forme
//! d'un graphe prêt à dessiner (diagramme de flux).
//!
//! Chaque visite fournit la suite de ses pages ; les rechargements consécutifs d'une
//! même page comptent pour une seule. On garde les `depth` premières pages, depuis
//! l'entrée sur le site ou depuis la première occurrence de la page de départ demandée.

use std::collections::HashMap;

use serde::Serialize;

pub const OTHERS: &str = "__autres__";
pub const EXIT: &str = "__sortie__";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Node {
    pub label: String,
    pub sessions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Link {
    pub level: usize,
    pub from: String,
    pub to: String,
    pub sessions: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PathsResult {
    pub sessions_in_period: u64,
    /// Visites qui contiennent la page de départ (toutes, s'il n'y en a pas).
    pub matched: u64,
    pub levels: Vec<Vec<Node>>,
    pub links: Vec<Link>,
}

pub struct PathCalculator {
    depth: usize,
    max_nodes: usize,
    start: Option<String>,
    sessions: u64,
    matched: u64,
    nodes: Vec<HashMap<String, u64>>,
    links: Vec<HashMap<(String, String), u64>>,
    current: Option<i64>,
    sequence: Vec<String>,
}

impl PathCalculator {
    /// `depth` : niveaux affichés (2 à 8). `max_nodes` : pages gardées par niveau (2 à 15),
    /// les autres sont regroupées. `start` : chemin de la page de départ, ou None.
    pub fn new(depth: usize, max_nodes: usize, start: Option<String>) -> Self {
        let depth = depth.clamp(2, 8);
        Self {
            depth,
            max_nodes: max_nodes.clamp(2, 15),
            start: start.filter(|s| !s.is_empty()),
            sessions: 0,
            matched: 0,
            nodes: vec![HashMap::new(); depth],
            links: vec![HashMap::new(); depth],
            current: None,
            sequence: Vec::new(),
        }
    }

    /// Ajoute une page vue. Les pages doivent arriver groupées par session et triées par date.
    pub fn push(&mut self, session: i64, path: &str) {
        if self.current != Some(session) {
            self.close_session();
            self.current = Some(session);
        }
        if self.sequence.last().map(String::as_str) != Some(path) {
            self.sequence.push(path.to_string());
        }
    }

    fn close_session(&mut self) {
        if self.current.is_none() {
            return;
        }
        self.sessions += 1;
        let mut seq = std::mem::take(&mut self.sequence);
        if let Some(start) = &self.start {
            match seq.iter().position(|p| p == start) {
                Some(pos) => {
                    seq.drain(..pos);
                }
                None => return,
            }
        }
        if seq.is_empty() {
            return;
        }
        self.matched += 1;
        let ended = seq.len() <= self.depth;
        seq.truncate(self.depth);
        for level in 0..seq.len() {
            *self.nodes[level].entry(seq[level].clone()).or_default() += 1;
            let to = if level + 1 < seq.len() {
                seq[level + 1].clone()
            } else if ended && level + 1 < self.depth {
                EXIT.to_string()
            } else {
                continue;
            };
            *self.links[level].entry((seq[level].clone(), to)).or_default() += 1;
        }
    }

    pub fn finish(mut self) -> PathsResult {
        self.close_session();

        // Par niveau : garder les pages les plus vues, regrouper le reste.
        let mut kept: Vec<Vec<String>> = Vec::new();
        let mut levels = Vec::new();
        for counts in &self.nodes {
            if counts.is_empty() {
                break;
            }
            let mut sorted: Vec<(&String, &u64)> = counts.iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
            let top = &sorted[..sorted.len().min(self.max_nodes)];
            let rest: u64 = sorted[top.len()..].iter().map(|(_, n)| **n).sum();
            let mut level: Vec<Node> =
                top.iter().map(|(label, n)| Node { label: (*label).clone(), sessions: **n }).collect();
            if rest > 0 {
                level.push(Node { label: OTHERS.into(), sessions: rest });
            }
            kept.push(top.iter().map(|(label, _)| (*label).clone()).collect());
            levels.push(level);
        }

        let mut merged: HashMap<(usize, String, String), u64> = HashMap::new();
        for (level, links) in self.links.iter().enumerate().take(kept.len()) {
            for ((from, to), n) in links {
                let from = if kept[level].contains(from) { from.clone() } else { OTHERS.to_string() };
                let to = if to == EXIT || kept.get(level + 1).is_some_and(|k| k.contains(to)) {
                    to.clone()
                } else {
                    OTHERS.to_string()
                };
                *merged.entry((level, from, to)).or_default() += n;
            }
        }
        let mut links: Vec<Link> =
            merged.into_iter().map(|((level, from, to), sessions)| Link { level, from, to, sessions }).collect();
        links.sort_by(|a, b| {
            a.level
                .cmp(&b.level)
                .then(b.sessions.cmp(&a.sessions))
                .then_with(|| a.from.cmp(&b.from))
                .then_with(|| a.to.cmp(&b.to))
        });
        PathsResult { sessions_in_period: self.sessions, matched: self.matched, levels, links }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(depth: usize, max_nodes: usize, start: Option<&str>, visits: &[&[&str]]) -> PathsResult {
        let mut calc = PathCalculator::new(depth, max_nodes, start.map(String::from));
        for (i, pages) in visits.iter().enumerate() {
            for page in *pages {
                calc.push(i as i64 + 1, page);
            }
        }
        calc.finish()
    }

    fn link(level: usize, from: &str, to: &str, sessions: u64) -> Link {
        Link { level, from: from.into(), to: to.into(), sessions }
    }

    #[test]
    fn levels_links_and_exits() {
        let r = run(3, 5, None, &[&["/", "/a", "/b", "/c"], &["/", "/a"], &["/", "/", "/b"], &["/x"]]);
        assert_eq!((r.sessions_in_period, r.matched), (4, 4));
        assert_eq!(
            r.levels[0],
            vec![Node { label: "/".into(), sessions: 3 }, Node { label: "/x".into(), sessions: 1 }]
        );
        assert_eq!(
            r.levels[1],
            vec![Node { label: "/a".into(), sessions: 2 }, Node { label: "/b".into(), sessions: 1 }]
        );
        assert_eq!(r.levels[2], vec![Node { label: "/b".into(), sessions: 1 }]);
        assert_eq!(
            r.links,
            vec![
                link(0, "/", "/a", 2),
                link(0, "/", "/b", 1), // le rechargement de « / » est ignoré
                link(0, "/x", EXIT, 1),
                link(1, "/a", "/b", 1),
                link(1, "/a", EXIT, 1),
                link(1, "/b", EXIT, 1),
                // La première visite continue au-delà du troisième niveau : pas de sortie comptée.
            ]
        );
    }

    #[test]
    fn groups_rare_pages_and_keeps_totals() {
        let visits: Vec<Vec<&str>> =
            vec![vec!["/", "/a"], vec!["/", "/a"], vec!["/", "/b"], vec!["/", "/c"], vec!["/", "/d"]];
        let refs: Vec<&[&str]> = visits.iter().map(Vec::as_slice).collect();
        let r = run(2, 2, None, &refs);
        assert_eq!(
            r.levels[1],
            vec![
                Node { label: "/a".into(), sessions: 2 },
                Node { label: "/b".into(), sessions: 1 },
                Node { label: OTHERS.into(), sessions: 2 }
            ]
        );
        assert_eq!(r.links, vec![link(0, "/", "/a", 2), link(0, "/", OTHERS, 2), link(0, "/", "/b", 1)]);
        assert_eq!(r.levels[1].iter().map(|n| n.sessions).sum::<u64>(), 5);
    }

    #[test]
    fn starts_from_a_given_page() {
        let r = run(2, 5, Some("/a"), &[&["/", "/a", "/b"], &["/a"], &["/", "/c"]]);
        assert_eq!((r.sessions_in_period, r.matched), (3, 2));
        assert_eq!(r.levels[0], vec![Node { label: "/a".into(), sessions: 2 }]);
        assert_eq!(r.links, vec![link(0, "/a", "/b", 1), link(0, "/a", EXIT, 1)]);
    }

    #[test]
    fn empty_input() {
        let r = run(5, 5, None, &[]);
        assert_eq!((r.sessions_in_period, r.matched), (0, 0));
        assert!(r.levels.is_empty() && r.links.is_empty());
    }
}
