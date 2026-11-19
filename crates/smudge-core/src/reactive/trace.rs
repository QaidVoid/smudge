//! What the devtools record of the reactive graph: where each signal, memo
//! and effect was defined, which write last woke each effect, how often each
//! effect ran, and the cycles a flush cut short.

use std::collections::VecDeque;
use std::panic::Location;
use std::rc::{Rc, Weak};

use slotmap::SecondaryMap;

use crate::inspect::{
    BindingInfo, CycleInfo, EffectRuns, OwnerEntry, SourceInfo, SourceKind, WriteInfo,
};

use super::{Node, NodeId, OwnerData, Runtime, Scope};

/// A place in the source.
pub(crate) type Site = &'static Location<'static>;

/// How many cut cycles the trace keeps, the oldest dropped first.
const CYCLES: usize = 32;

/// The source that changed and where it was written, as a write records it
/// while it wakes effects.
#[derive(Copy, Clone)]
pub(crate) struct Wake {
    /// The signal that was written.
    pub(crate) source: NodeId,
    /// Where the write happened.
    pub(crate) written_at: Site,
}

/// A cycle a flush cut short.
struct Cycle {
    /// The root scope of the effect, which tells the apps of a thread apart.
    /// It is held weakly, so a cycle of an app that is gone keeps nothing of
    /// it alive.
    scope: Weak<Scope>,
    /// What the inspector shows of it.
    info: CycleInfo,
}

/// The record itself, kept by the runtime with the `devtools` feature.
#[derive(Default)]
pub(crate) struct Trace {
    /// Where each node was defined, for the nodes whose constructor passed
    /// its caller on.
    defined: SecondaryMap<NodeId, Site>,
    /// How many times each effect's body ran.
    runs: SecondaryMap<NodeId, u64>,
    /// The write that last woke each effect.
    wakes: SecondaryMap<NodeId, Wake>,
    /// The write being propagated right now, if a write is.
    pub(crate) writing: Option<Wake>,
    /// Where the last write happened.
    pub(crate) last_write: Option<Site>,
    /// The cycles cut short, oldest first.
    cycles: VecDeque<Cycle>,
}

impl Trace {
    /// Records that `id` was defined at `at`.
    pub(crate) fn define(&mut self, id: NodeId, at: Site) {
        self.defined.insert(id, at);
    }

    /// Records that the effect `id` was woken by the write being
    /// propagated, if one is.
    pub(crate) fn woken(&mut self, id: NodeId) {
        if let Some(wake) = self.writing {
            self.wakes.insert(id, wake);
        }
    }

    /// Counts one run of the effect `id`.
    pub(crate) fn ran(&mut self, id: NodeId) {
        if let Some(runs) = self.runs.entry(id).map(|entry| entry.or_insert(0)) {
            *runs += 1;
        }
    }

    /// Forgets the node `id`, which was disposed.
    pub(crate) fn forget(&mut self, id: NodeId) {
        self.defined.remove(id);
        self.runs.remove(id);
        self.wakes.remove(id);
    }

    /// Returns where `id` was defined.
    fn defined_at(&self, id: NodeId) -> Option<Site> {
        self.defined.get(id).copied()
    }
}

impl Runtime {
    /// Records that the effect `id` of `scope` ran too often in one flush,
    /// keeps the record and prints it to stderr.
    pub(crate) fn trace_cycle(&mut self, id: NodeId, scope: Rc<Scope>) {
        let info = CycleInfo {
            effect_defined_at: self.trace.defined_at(id),
            written_at: self.trace.wakes.get(id).map(|wake| wake.written_at),
        };
        eprintln!(
            "smudge devtools: a binding {} ran too often in one flush, last woken by the write {}",
            site_text("defined", info.effect_defined_at),
            site_text("at", info.written_at),
        );
        if self.trace.cycles.len() == CYCLES {
            self.trace.cycles.pop_front();
        }
        self.trace.cycles.push_back(Cycle {
            scope: Rc::downgrade(&scope),
            info,
        });
    }

    /// Returns what the inspector shows of each effect `owner` holds.
    pub(crate) fn bindings_of(&self, owner: &OwnerData) -> Vec<BindingInfo> {
        let state = owner.state.borrow();
        state
            .nodes
            .iter()
            .filter_map(|&id| match self.arena.get(id) {
                Some(Node::Effect { sources, .. }) => Some(BindingInfo {
                    defined_at: self.trace.defined_at(id),
                    runs: self.trace.runs.get(id).copied().unwrap_or(0),
                    sources: sources
                        .iter()
                        .filter_map(|&(source, _)| {
                            let kind = match self.arena.get(source)? {
                                Node::Signal { .. } => SourceKind::Signal,
                                Node::Memo { .. } => SourceKind::Memo,
                                Node::Effect { .. } => return None,
                            };
                            Some(SourceInfo {
                                kind,
                                defined_at: self.trace.defined_at(source),
                            })
                        })
                        .collect(),
                    woken_by: self.trace.wakes.get(id).map(|wake| WriteInfo {
                        source_defined_at: self.trace.defined_at(wake.source),
                        written_at: wake.written_at,
                    }),
                }),
                _ => None,
            })
            .collect()
    }

    /// Returns the `limit` effects of `scope` that ran most, most first.
    pub(crate) fn busiest(&self, scope: &Rc<Scope>, limit: usize) -> Vec<EffectRuns> {
        let mut effects: Vec<EffectRuns> = self
            .arena
            .iter()
            .filter_map(|(id, node)| match node {
                Node::Effect { scope: own, .. } if Rc::ptr_eq(own, scope) => Some(EffectRuns {
                    defined_at: self.trace.defined_at(id),
                    runs: self.trace.runs.get(id).copied().unwrap_or(0),
                }),
                _ => None,
            })
            .collect();
        effects.sort_by_key(|effect| std::cmp::Reverse(effect.runs));
        effects.truncate(limit);
        effects
    }

    /// Records that a frame of `scope` stopped applying updates at its
    /// round cap, with the last write as its cause, keeps the record and
    /// prints it to stderr.
    pub(crate) fn trace_round_cycle(&mut self, scope: Rc<Scope>) {
        let info = CycleInfo {
            effect_defined_at: None,
            written_at: self.trace.last_write,
        };
        eprintln!(
            "smudge devtools: a frame's updates did not settle, last woken by the write {}",
            site_text("at", info.written_at),
        );
        if self.trace.cycles.len() == CYCLES {
            self.trace.cycles.pop_front();
        }
        self.trace.cycles.push_back(Cycle {
            scope: Rc::downgrade(&scope),
            info,
        });
    }

    /// Returns the cycles of `scope` a flush cut short, oldest first.
    pub(crate) fn cycles(&self, scope: &Rc<Scope>) -> Vec<CycleInfo> {
        self.trace
            .cycles
            .iter()
            .filter(|cycle| std::ptr::eq(cycle.scope.as_ptr(), Rc::as_ptr(scope)))
            .map(|cycle| cycle.info)
            .collect()
    }

    /// Calls `visit` with `owner` and every owner under it, parents first,
    /// with what each holds.
    pub(crate) fn visit_owners(
        &self,
        owner: &OwnerData,
        depth: usize,
        visit: &mut dyn FnMut(OwnerEntry),
    ) {
        let state = owner.state.borrow();
        let mut entry = OwnerEntry {
            depth,
            signals: 0,
            memos: 0,
            effects: 0,
        };
        for &id in &state.nodes {
            match self.arena.get(id) {
                Some(Node::Signal { .. }) => entry.signals += 1,
                Some(Node::Memo { .. }) => entry.memos += 1,
                Some(Node::Effect { .. }) => entry.effects += 1,
                None => {}
            }
        }
        visit(entry);
        let children: Vec<Rc<OwnerData>> = state
            .children
            .iter()
            .filter_map(std::rc::Weak::upgrade)
            .collect();
        drop(state);
        for child in children {
            self.visit_owners(&child, depth + 1, visit);
        }
    }
}

/// Writes `site` after `word`, or says it is unknown.
fn site_text(word: &str, site: Option<Site>) -> String {
    site.map_or_else(
        || String::from("(place unknown)"),
        |site| format!("{word} {}:{}:{}", site.file(), site.line(), site.column()),
    )
}
