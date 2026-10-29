//! Provider evidence voting; thresholds are conservative engineering policy, not accuracy claims.
use super::contracts::*;
use crate::ai::speech::StreamIdentity;
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

pub const VOTE_WINDOWS: usize = 5;
pub const REQUIRED_VOTES: usize = 3;
pub const SONG_CONFIDENCE: u16 = 800;
pub const CLASSIFICATION_CONFIDENCE: u16 = 700;
pub const AMBIGUITY_MARGIN: u16 = 100;
pub const MAX_MISSES: usize = 2;
pub const VOTE_FRESHNESS: Duration = Duration::from_secs(15);
pub const CONFIRMED_FRESHNESS: Duration = Duration::from_secs(6);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SongState {
    Unknown,
    Candidate,
    Confirmed,
    Transition,
}
#[derive(Clone)]
pub struct Resolution {
    pub kind: ContentType,
    pub song_state: SongState,
    pub song: Option<Arc<SongCandidate>>,
    pub movies: Vec<Arc<MovieCandidate>>,
}
impl Default for Resolution {
    fn default() -> Self {
        Self {
            kind: ContentType::Unknown,
            song_state: SongState::Unknown,
            song: None,
            movies: Vec::new(),
        }
    }
}
struct Vote {
    at: Instant,
    id: Option<Box<str>>,
}
pub struct CandidateTracker {
    identity: StreamIdentity,
    previous: Option<(u64, Instant)>,
    votes: VecDeque<Vote>,
    incumbent: Option<(Arc<SongCandidate>, Instant)>,
    misses: usize,
    transitioning: bool,
}
impl CandidateTracker {
    pub fn new(identity: StreamIdentity) -> Self {
        Self {
            identity,
            previous: None,
            votes: VecDeque::new(),
            incumbent: None,
            misses: 0,
            transitioning: false,
        }
    }
    pub fn clear(&mut self) {
        self.votes.clear();
        self.incumbent = None;
        self.misses = 0;
        self.transitioning = false;
    }
    pub fn observe(
        &mut self,
        stamp: WindowStamp,
        evidence: ContentEvidence,
        now: Instant,
    ) -> Result<Resolution, ResolverError> {
        stamp.validate(self.identity, now)?;
        evidence.validate()?;
        if self
            .previous
            .is_some_and(|(seq, time)| stamp.sequence <= seq || stamp.ended_at < time)
        {
            return Err(ResolverError::Stale);
        }
        self.previous = Some((stamp.sequence, stamp.ended_at));
        self.votes.retain(|vote| {
            now.checked_duration_since(vote.at)
                .is_some_and(|age| age <= VOTE_FRESHNESS)
        });
        if self.incumbent.as_ref().is_some_and(|(_, at)| {
            now.checked_duration_since(*at)
                .is_none_or(|age| age > CONFIRMED_FRESHNESS)
        }) {
            self.incumbent = None;
        }
        let kind = if evidence.confidence.permille() >= CLASSIFICATION_CONFIDENCE {
            evidence.kind
        } else {
            ContentType::Unknown
        };
        if !matches!(kind, ContentType::Music | ContentType::Unknown) {
            self.clear();
            return Ok(Resolution {
                kind,
                song_state: if kind == ContentType::Mixed {
                    SongState::Transition
                } else {
                    SongState::Unknown
                },
                song: None,
                movies: evidence.movies.into_iter().map(Arc::new).collect(),
            });
        }
        let mut songs = evidence.songs;
        songs.sort_by(|a, b| {
            b.confidence
                .cmp(&a.confidence)
                .then_with(|| a.id.cmp(&b.id))
        });
        if kind == ContentType::Music
            && songs
                .first()
                .is_some_and(|song| song.confidence.permille() >= SONG_CONFIDENCE)
            && songs.get(1).is_some_and(|second| {
                songs[0]
                    .confidence
                    .permille()
                    .saturating_sub(second.confidence.permille())
                    < AMBIGUITY_MARGIN
            })
        {
            self.transitioning = true;
        }
        let top = songs
            .first()
            .filter(|song| {
                kind == ContentType::Music
                    && song.confidence.permille() >= SONG_CONFIDENCE
                    && songs.get(1).is_none_or(|second| {
                        song.confidence
                            .permille()
                            .saturating_sub(second.confidence.permille())
                            >= AMBIGUITY_MARGIN
                    })
            })
            .cloned()
            .map(Arc::new);
        if self.votes.len() == VOTE_WINDOWS {
            self.votes.pop_front();
        }
        self.votes.push_back(Vote {
            at: stamp.ended_at,
            id: top.as_ref().map(|song| song.id.clone()),
        });
        let Some(top) = top else {
            self.misses += 1;
            if self.misses >= MAX_MISSES {
                self.incumbent = None;
            }
            if !self.transitioning
                && let Some((song, _)) = &self.incumbent
            {
                return Ok(Resolution {
                    kind: ContentType::Music,
                    song_state: SongState::Confirmed,
                    song: Some(song.clone()),
                    movies: Vec::new(),
                });
            }
            return Ok(Resolution {
                kind: if self.transitioning {
                    ContentType::Mixed
                } else {
                    kind
                },
                song_state: if self.transitioning {
                    SongState::Transition
                } else {
                    SongState::Unknown
                },
                song: None,
                movies: Vec::new(),
            });
        };
        self.misses = 0;
        let matching = self
            .votes
            .iter()
            .filter(|vote| vote.id.as_deref() == Some(top.id.as_ref()))
            .count();
        if matching >= REQUIRED_VOTES
            || self
                .incumbent
                .as_ref()
                .is_some_and(|(song, _)| song.id == top.id)
        {
            self.incumbent = Some((top.clone(), stamp.ended_at));
            self.transitioning = false;
            return Ok(Resolution {
                kind: ContentType::Music,
                song_state: SongState::Confirmed,
                song: Some(top),
                movies: Vec::new(),
            });
        }
        if self.incumbent.is_some() || self.transitioning {
            self.transitioning = true;
            Ok(Resolution {
                kind: ContentType::Mixed,
                song_state: SongState::Transition,
                song: None,
                movies: Vec::new(),
            })
        } else {
            Ok(Resolution {
                kind,
                song_state: SongState::Candidate,
                song: Some(top),
                movies: Vec::new(),
            })
        }
    }
}
