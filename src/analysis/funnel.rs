//! Entonnoirs : combien de visites franchissent une suite d'étapes, où elles abandonnent
//! et en combien de temps elles passent d'une étape à la suivante.
//!
//! Règle : une visite franchit l'étape N si, après l'événement qui a validé l'étape N-1,
//! elle contient un événement qui correspond à l'étape N. D'autres événements peuvent
//! s'intercaler. Chaque étape prend le premier événement possible, ce qui maximise le
//! nombre d'étapes franchies.

use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

pub const MIN_STEPS: usize = 2;
pub const MAX_STEPS: usize = 12;
/// Visites gardées en exemple par étape, pour aller voir qui a abandonné.
const SAMPLE_SIZE: usize = 50;
const NEXT_PAGES: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Une page vue, comparée par son chemin.
    Page,
    /// Un événement personnalisé, comparé par son nom.
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchOp {
    Equals,
    Contains,
    StartsWith,
    EndsWith,
    Regex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepDef {
    #[serde(default)]
    pub name: String,
    pub kind: StepKind,
    #[serde(rename = "match", default = "default_op")]
    pub op: MatchOp,
    pub pattern: String,
}

fn default_op() -> MatchOp {
    MatchOp::Equals
}

/// Valide et normalise des étapes saisies par un utilisateur. Le message d'erreur lui est destiné.
pub fn sanitize(steps: Vec<StepDef>) -> Result<Vec<StepDef>, String> {
    if steps.len() < MIN_STEPS {
        return Err("Un entonnoir compte au moins deux étapes.".into());
    }
    if steps.len() > MAX_STEPS {
        return Err(format!("Un entonnoir compte au plus {MAX_STEPS} étapes."));
    }
    steps
        .into_iter()
        .enumerate()
        .map(|(i, step)| {
            let n = i + 1;
            let pattern: String = step.pattern.trim().chars().filter(|c| !c.is_control()).take(500).collect();
            if pattern.is_empty() {
                return Err(format!("Étape {n} : le motif est vide."));
            }
            if step.op == MatchOp::Regex && Regex::new(&pattern).is_err() {
                return Err(format!("Étape {n} : expression régulière invalide."));
            }
            let name: String = step.name.trim().chars().filter(|c| !c.is_control()).take(80).collect();
            let name = if name.is_empty() { pattern.chars().take(80).collect() } else { name };
            Ok(StepDef { name, pattern, ..step })
        })
        .collect()
}

enum Test {
    Equals(String),
    Contains(String),
    StartsWith(String),
    EndsWith(String),
    /// Le moteur `regex` garantit un temps linéaire : un motif saisi ne peut pas bloquer le serveur.
    Regex(Regex),
}

struct Step {
    kind: StepKind,
    test: Test,
}

/// Événement vu par les moteurs d'analyse.
#[derive(Debug, Clone, Copy)]
pub struct Hit<'a> {
    pub session: i64,
    pub ts_ms: i64,
    pub pageview: bool,
    pub path: &'a str,
    pub name: Option<&'a str>,
}

fn trim_slash(path: &str) -> &str {
    if path.len() > 1 { path.trim_end_matches('/') } else { path }
}

impl Step {
    fn compile(def: &StepDef) -> Result<Self, String> {
        let pattern = def.pattern.clone();
        let test = match def.op {
            MatchOp::Equals => Test::Equals(pattern),
            MatchOp::Contains => Test::Contains(pattern),
            MatchOp::StartsWith => Test::StartsWith(pattern),
            MatchOp::EndsWith => Test::EndsWith(pattern),
            MatchOp::Regex => {
                Test::Regex(Regex::new(&pattern).map_err(|_| "Expression régulière invalide.".to_string())?)
            }
        };
        Ok(Self { kind: def.kind, test })
    }

    fn matches(&self, hit: &Hit) -> bool {
        let value = match (self.kind, hit.pageview) {
            (StepKind::Page, true) => hit.path,
            (StepKind::Event, false) => hit.name.unwrap_or(""),
            _ => return false,
        };
        match &self.test {
            // « /panier » et « /panier/ » désignent la même page.
            Test::Equals(p) if self.kind == StepKind::Page => trim_slash(value) == trim_slash(p),
            Test::Equals(p) => value == p,
            Test::Contains(p) => value.contains(p.as_str()),
            Test::StartsWith(p) => value.starts_with(p.as_str()),
            Test::EndsWith(p) => value.ends_with(p.as_str()),
            Test::Regex(r) => r.is_match(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NextPage {
    /// Vrai : la visite s'est arrêtée là. Faux : elle a continué sur `path`.
    pub exit: bool,
    pub path: String,
    pub sessions: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StepResult {
    pub name: String,
    pub sessions: u64,
    /// Visites qui ont franchi cette étape mais pas la suivante.
    pub dropped: u64,
    pub rate_from_start: f64,
    pub rate_from_previous: f64,
    /// Temps médian depuis l'étape précédente, en secondes.
    pub median_seconds_from_previous: Option<u64>,
    /// Où sont allées les visites qui ont abandonné après cette étape.
    pub next_after_drop: Vec<NextPage>,
    pub dropped_sessions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FunnelResult {
    pub sessions_in_period: u64,
    pub entered: u64,
    pub converted: u64,
    pub conversion_rate: f64,
    pub steps: Vec<StepResult>,
}

#[derive(Default)]
struct Tally {
    reached: u64,
    gaps: Vec<u64>,
    exits: u64,
    next: HashMap<String, u64>,
    sample: Vec<i64>,
}

pub struct FunnelCalculator {
    names: Vec<String>,
    steps: Vec<Step>,
    tallies: Vec<Tally>,
    sessions: u64,
    current: Option<i64>,
    index: usize,
    last_match_ms: i64,
    awaiting_next: bool,
    next_page: Option<String>,
}

impl FunnelCalculator {
    pub fn new(defs: &[StepDef]) -> Result<Self, String> {
        let steps = defs.iter().map(Step::compile).collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            names: defs.iter().map(|d| d.name.clone()).collect(),
            tallies: steps.iter().map(|_| Tally::default()).collect(),
            steps,
            sessions: 0,
            current: None,
            index: 0,
            last_match_ms: 0,
            awaiting_next: false,
            next_page: None,
        })
    }

    /// Les événements doivent arriver groupés par session et triés par date.
    pub fn push(&mut self, hit: &Hit) {
        if self.current != Some(hit.session) {
            self.close_session();
            self.current = Some(hit.session);
            self.sessions += 1;
            self.index = 0;
            self.last_match_ms = 0;
            self.awaiting_next = false;
            self.next_page = None;
        }
        if self.index < self.steps.len() && self.steps[self.index].matches(hit) {
            let tally = &mut self.tallies[self.index];
            tally.reached += 1;
            if self.index > 0 {
                tally.gaps.push(((hit.ts_ms - self.last_match_ms).max(0) / 1000) as u64);
            }
            self.last_match_ms = hit.ts_ms;
            self.index += 1;
            self.awaiting_next = true;
            self.next_page = None;
            return;
        }
        // Première page vue après la dernière étape franchie : dit où partent ceux qui abandonnent.
        if self.awaiting_next && hit.pageview {
            self.next_page = Some(hit.path.to_string());
            self.awaiting_next = false;
        }
    }

    fn close_session(&mut self) {
        let Some(session) = self.current else { return };
        if self.index == 0 || self.index >= self.steps.len() {
            return;
        }
        let tally = &mut self.tallies[self.index - 1];
        match self.next_page.take() {
            Some(path) => *tally.next.entry(path).or_default() += 1,
            None => tally.exits += 1,
        }
        if tally.sample.len() < SAMPLE_SIZE {
            tally.sample.push(session);
        }
    }

    pub fn finish(mut self) -> FunnelResult {
        self.close_session();
        let entered = self.tallies.first().map_or(0, |t| t.reached);
        let converted = self.tallies.last().map_or(0, |t| t.reached);
        let ratio = |a: u64, b: u64| if b > 0 { a as f64 / b as f64 } else { 0.0 };
        let reached: Vec<u64> = self.tallies.iter().map(|t| t.reached).collect();
        let steps = self
            .tallies
            .into_iter()
            .enumerate()
            .map(|(i, mut tally)| {
                let mut next: Vec<NextPage> =
                    tally.next.into_iter().map(|(path, sessions)| NextPage { exit: false, path, sessions }).collect();
                if tally.exits > 0 {
                    next.push(NextPage { exit: true, path: String::new(), sessions: tally.exits });
                }
                // Tri déterministe : les plus fréquents d'abord, puis par chemin.
                next.sort_by(|a, b| b.sessions.cmp(&a.sessions).then_with(|| a.path.cmp(&b.path)));
                next.truncate(NEXT_PAGES);
                StepResult {
                    name: self.names[i].clone(),
                    sessions: tally.reached,
                    dropped: reached.get(i + 1).map_or(0, |next| tally.reached - next),
                    rate_from_start: ratio(tally.reached, entered),
                    rate_from_previous: if i == 0 { 1.0 } else { ratio(tally.reached, reached[i - 1]) },
                    median_seconds_from_previous: median(&mut tally.gaps),
                    next_after_drop: next,
                    // En texte : un identifiant sur 63 bits dépasse la précision des nombres JavaScript.
                    dropped_sessions: tally.sample.iter().map(i64::to_string).collect(),
                }
            })
            .collect();
        FunnelResult {
            sessions_in_period: self.sessions,
            entered,
            converted,
            conversion_rate: ratio(converted, entered),
            steps,
        }
    }
}

fn median(values: &mut [u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    Some(if values.len() % 2 == 1 { values[mid] } else { (values[mid - 1] + values[mid]).div_ceil(2) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: StepKind, op: MatchOp, pattern: &str) -> StepDef {
        StepDef { name: pattern.into(), kind, op, pattern: pattern.into() }
    }

    fn page(session: i64, seconds: i64, path: &'static str) -> Hit<'static> {
        Hit { session, ts_ms: seconds * 1000, pageview: true, path, name: None }
    }

    fn event(session: i64, seconds: i64, name: &'static str) -> Hit<'static> {
        Hit { session, ts_ms: seconds * 1000, pageview: false, path: "/x", name: Some(name) }
    }

    fn run(defs: &[StepDef], hits: &[Hit]) -> FunnelResult {
        let mut calc = FunnelCalculator::new(defs).unwrap();
        for hit in hits {
            calc.push(hit);
        }
        calc.finish()
    }

    #[test]
    fn counts_steps_drops_and_medians() {
        let defs = [
            step(StepKind::Page, MatchOp::Equals, "/"),
            step(StepKind::Page, MatchOp::StartsWith, "/produits/"),
            step(StepKind::Event, MatchOp::Equals, "Ajout"),
        ];
        let hits = [
            // Session 1 : va au bout, avec des pages intercalées.
            page(1, 0, "/"),
            page(1, 10, "/blog"),
            page(1, 30, "/produits/lampe"),
            event(1, 50, "Ajout"),
            // Session 2 : s'arrête après l'accueil, repart vers /contact.
            page(2, 0, "/"),
            page(2, 5, "/contact"),
            // Session 3 : s'arrête après l'accueil sans autre page.
            page(3, 0, "/"),
            // Session 4 : n'entre jamais (pas d'accueil).
            page(4, 0, "/produits/lampe"),
            event(4, 5, "Ajout"),
            // Session 5 : fiche produit en 90 s, puis abandon vers le catalogue.
            page(5, 0, "/"),
            page(5, 90, "/produits/chaise"),
            page(5, 95, "/produits"),
        ];
        let r = run(&defs, &hits);
        assert_eq!(r.sessions_in_period, 5);
        assert_eq!(r.steps.iter().map(|s| s.sessions).collect::<Vec<_>>(), vec![4, 2, 1]);
        assert_eq!(r.steps.iter().map(|s| s.dropped).collect::<Vec<_>>(), vec![2, 1, 0]);
        assert_eq!((r.entered, r.converted), (4, 1));
        assert!((r.conversion_rate - 0.25).abs() < 1e-12);
        assert_eq!(r.steps[1].median_seconds_from_previous, Some(60), "médiane de 30 s et 90 s");
        assert_eq!(r.steps[2].median_seconds_from_previous, Some(20));
        assert_eq!(r.steps[0].median_seconds_from_previous, None);
        assert_eq!(
            r.steps[0].next_after_drop,
            vec![
                // À égalité, l'ordre est celui des chemins : la sortie (chemin vide) vient d'abord.
                NextPage { exit: true, path: String::new(), sessions: 1 },
                NextPage { exit: false, path: "/contact".into(), sessions: 1 },
            ]
        );
        assert_eq!(r.steps[1].next_after_drop, vec![NextPage { exit: false, path: "/produits".into(), sessions: 1 }]);
        assert_eq!(r.steps[0].dropped_sessions, vec!["2", "3"]);
    }

    #[test]
    fn an_event_never_validates_two_steps_and_order_matters() {
        let defs = [step(StepKind::Page, MatchOp::Contains, "/a"), step(StepKind::Page, MatchOp::Contains, "/a")];
        assert_eq!(run(&defs, &[page(1, 0, "/a")]).steps[1].sessions, 0);
        assert_eq!(run(&defs, &[page(1, 0, "/a"), page(1, 1, "/a")]).steps[1].sessions, 1);
        let ordered = [step(StepKind::Page, MatchOp::Equals, "/un"), step(StepKind::Page, MatchOp::Equals, "/deux")];
        assert_eq!(
            run(&ordered, &[page(1, 0, "/deux"), page(1, 1, "/un")]).converted,
            0,
            "étapes dans le mauvais ordre"
        );
    }

    #[test]
    fn matching_rules() {
        let hit = page(1, 0, "/panier/");
        let ok = |kind, op, pattern: &str| Step::compile(&step(kind, op, pattern)).unwrap().matches(&hit);
        assert!(ok(StepKind::Page, MatchOp::Equals, "/panier"), "barre finale tolérée");
        assert!(!ok(StepKind::Page, MatchOp::Equals, "/Panier"), "la casse compte");
        assert!(ok(StepKind::Page, MatchOp::EndsWith, "nier/"));
        assert!(ok(StepKind::Page, MatchOp::Regex, "^/pan(ier|ama)/?$"));
        assert!(!ok(StepKind::Event, MatchOp::Contains, "panier"), "une étape d'événement ignore les pages");
        let ev = event(1, 0, "Ajout au panier");
        assert!(Step::compile(&step(StepKind::Event, MatchOp::StartsWith, "Ajout")).unwrap().matches(&ev));
        assert!(!Step::compile(&step(StepKind::Page, MatchOp::Contains, "/x")).unwrap().matches(&ev));
    }

    #[test]
    fn sanitizes_user_steps() {
        let two = vec![
            step(StepKind::Page, MatchOp::Equals, " / "),
            StepDef { name: String::new(), ..step(StepKind::Event, MatchOp::Equals, "Ajout") },
        ];
        let clean = sanitize(two.clone()).unwrap();
        assert_eq!(clean[0].pattern, "/");
        assert_eq!(clean[1].name, "Ajout", "le nom par défaut est le motif");
        assert!(sanitize(two[..1].to_vec()).unwrap_err().contains("au moins deux"));
        assert!(sanitize(vec![two[0].clone(); 13]).unwrap_err().contains("au plus"));
        assert!(
            sanitize(vec![two[0].clone(), step(StepKind::Page, MatchOp::Regex, "(")])
                .unwrap_err()
                .contains("régulière")
        );
        assert!(
            sanitize(vec![two[0].clone(), step(StepKind::Page, MatchOp::Equals, "  ")]).unwrap_err().contains("vide")
        );
        let parsed: StepDef = serde_json::from_str(r#"{"kind":"page","match":"starts_with","pattern":"/p"}"#).unwrap();
        assert_eq!((parsed.kind, parsed.op), (StepKind::Page, MatchOp::StartsWith));
        assert!(serde_json::from_str::<StepDef>(r#"{"kind":"autre","pattern":"/p"}"#).is_err());
    }

    #[test]
    fn median_of_even_and_odd() {
        assert_eq!(median(&mut []), None);
        assert_eq!(median(&mut [5]), Some(5));
        assert_eq!(median(&mut [9, 1, 5]), Some(5));
        assert_eq!(median(&mut [1, 4]), Some(3), "moyenne arrondie au supérieur");
    }
}
