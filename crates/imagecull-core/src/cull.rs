//! The cull: every group of a shoot, the keeper clef chose, and the red on
//! everyone else.
//!
//! `imagecull groups` finds the groups and measures them; this module is what
//! turns that plan into a decision. One comparative request per round asks which
//! frame of a group to keep, the answer names one, and every other frame of the
//! group gets the red mark a cull can filter on. A group of one is asked nothing
//! and marked nothing — but a red this tool wrote on a frame that is no longer in
//! a group is retired, because the group it lost does not exist any more.
//!
//! # What it reads and what it never writes
//!
//! The comparison sees the frames' pixels and nothing else. A frame's
//! `xmp:Rating` is not read, not used to shortlist whom clef is shown, and not
//! written: the per-frame rating is a weaker judgement than the group's, and
//! letting it choose who is compared would let it veto the answer. A sidecar this
//! module writes carries the verdict, the label and their provenance, and nothing
//! about a rating.
//!
//! # Whose label it is
//!
//! The rules of ADR 0007 hold: a photographer's Yellow is never overwritten, a
//! label without our `labeledBy` proof is never removed, and a group nothing
//! could decide marks nothing. ADR 0008 adds the one exception: a red this tool
//! wrote is retired when a later run promotes that frame to its group's keeper,
//! because the cull is repeatable and a red on the keeper is the one thing it
//! must not leave.
//!
//! # Nothing here knows a terminal or a command line
//!
//! The pass reports what it did through a [`Watcher`] and returns counts; the
//! command renders them. The model call goes through
//! [`Decider::compare`](crate::scoring::Decider::compare), so a whole cull runs
//! in a test with no server.

use crate::error::{Error, ErrorKind, Result};
use crate::group::{GroupRow, Plan, Planned};
use crate::keeper::{self, Budget, Comparison};
use crate::preprocess::Params;
use crate::scoring::{self, Decider, State};
use crate::sidecar::{self, Failure, Mark, Prior, Writer};

/// What one cull pass did, group by group and in total.
///
/// Every count counts frames except `groups`, `rounds`, `low_confidence` and
/// `undecided`, which count groups or requests. The three that describe
/// non-keepers — `non_keepers`, `red` and `unmarked` — count the same
/// population, so `non_keepers` is `red` plus `unmarked` and the summary's
/// arithmetic adds up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Culled {
    /// How many groups of two or more frames were judged.
    pub groups: usize,
    /// How many non-keepers carry this run's verdict.
    pub non_keepers: usize,
    /// How many of them are red.
    pub red: usize,
    /// How many were left unmarked because the label on the frame is not ours to
    /// write.
    pub unmarked: usize,
    /// How many keepers lost the red an earlier run of this tool wrote.
    pub retired: usize,
    /// How many keepers still wear a red this tool did not write.
    pub stale_reds: usize,
    /// How many groups the comparison was less sure of than [`keeper::FLOOR`],
    /// and marked anyway.
    pub low_confidence: usize,
    /// How many groups no comparison could decide.
    pub undecided: usize,
    /// How many frames a verdict could not be written to.
    pub unwritten: usize,
    /// How many batches the pass tried to compare: the requests a decided group
    /// spent, and the ones an undecided group spent before it failed.
    pub rounds: usize,
    /// One entry per group judged, in capture order: what the report prints and
    /// what a reader checks the totals against.
    pub verdicts: Vec<Verdict>,
}

/// One group's verdict, as the report and the sidecar both describe it.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// The group's number, from 1, in capture order.
    pub number: usize,
    /// The name the group is reported under: the frame it opens with.
    pub start: String,
    /// How many frames the group holds.
    pub size: usize,
    /// The frame clef chose, by the name the folder knows it by; `None` when the
    /// group is undecided.
    pub keeper: Option<String>,
    /// How many non-keepers this group marked.
    pub red: usize,
    /// How many it left to the photographer.
    pub unmarked: usize,
    /// The shakiest call the verdict depended on.
    pub confidence: f64,
    /// How many comparative requests stood between the group and this verdict.
    pub rounds: usize,
    /// Why the group has no keeper, when it has none.
    pub failure: Option<Failure>,
}

/// What a cull pass needs to know: which model, how much picture one request
/// carries, and where the verdict's provenance comes from.
#[derive(Debug, Clone)]
pub struct Config<'a> {
    /// The alias the comparison asks.
    pub model: &'a str,
    /// The photographer's framing of the shoot, when they gave one.
    pub shoot_context: Option<String>,
    /// The measured request shape: edge, cap and confidence floor.
    pub budget: Budget,
    /// The JPEG quality a comparison preview is encoded at. Its edge is the
    /// budget's, because that is the measured number and a caller cannot choose
    /// how much picture the model sees.
    pub jpeg_quality: u8,
    /// The grouping hash the groups were computed under, written with every
    /// verdict.
    pub grouping_hash: &'a str,
    /// Whether a sidecar another application owns may be written into.
    pub allow_foreign: bool,
    /// How hard to retry a transient comparison failure.
    pub retry: crate::scoring::RetryPolicy,
}

/// What a cull pass tells a watcher while it runs.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The pass beginning: how many groups it faces.
    Started {
        /// How many groups of two or more are judged: the requests' denominator.
        groups: usize,
    },
    /// One group judged, or reported undecided.
    Group(Verdict),
    /// A verdict that could not reach its sidecar, and why: silence would make
    /// the delete set look complete.
    Unwritten {
        /// The frame whose verdict is missing.
        filename: String,
        /// The failure, as the photographer reads it.
        message: String,
    },
}

/// The port a pass reports through: the terminal in production, a recorder in a
/// test, nothing at all in [`Silent`].
pub trait Watcher {
    /// One event of the pass.
    fn event(&self, event: &Event);
}

/// A watcher with nothing to say, for a caller that does not render the pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct Silent;

impl Watcher for Silent {
    fn event(&self, _event: &Event) {}
}

impl Culled {
    /// Record a verdict that did not reach its sidecar, and name the frame and
    /// the reason: silence about a missing mark would make the delete set look
    /// complete.
    fn unwritten(&mut self, frame: &Planned, message: impl Into<String>, watcher: &dyn Watcher) {
        self.unwritten += 1;
        watcher.event(&Event::Unwritten {
            filename: name(frame),
            message: message.into(),
        });
    }
}

/// Judge every group in `plan` and write what it decided.
///
/// The preflight runs first, so a server that cannot answer a comparative
/// request costs seconds rather than a whole shoot's requests: nothing is marked
/// until the first group is answered. A group nothing could decide marks
/// nothing; the rest of the folder is judged regardless.
pub fn run(
    plan: &Plan,
    config: &Config<'_>,
    decider: &dyn Decider,
    writer: &dyn Writer,
    watcher: &dyn Watcher,
) -> Result<Culled> {
    let rows = plan.grouped();
    let groups = rows.iter().filter(|row| row.frames.len() > 1).count();
    watcher.event(&Event::Started { groups });
    // A folder with no group asks no comparative question, so it needs no
    // server: the preflight is about the requests the pass will make, and a
    // shoot of singletons makes none.
    if groups > 0 {
        decider.preflight()?;
    }
    let mut culled = Culled::default();
    for row in &rows {
        match row.frames {
            [only] => retire_singleton(only, config, writer, watcher, &mut culled),
            _ => judge(row, config, decider, writer, watcher, &mut culled),
        }
    }
    Ok(culled)
}

/// One group of two or more: compare its frames in rounds, then mark every
/// member of the group with what that decided.
fn judge(
    row: &GroupRow<'_>,
    config: &Config<'_>,
    decider: &dyn Decider,
    writer: &dyn Writer,
    watcher: &dyn Watcher,
    culled: &mut Culled,
) {
    let mut verdict = Verdict {
        number: row.number,
        start: name(&row.frames[0]),
        size: row.frames.len(),
        keeper: None,
        red: 0,
        unmarked: 0,
        confidence: 1.0,
        rounds: 0,
        failure: None,
    };
    culled.groups += 1;
    let chosen = choose(row, config, decider, &mut verdict);
    // The requests are spent whether or not they decided anything: a group that
    // failed on its second batch still cost one, and the summary counts what the
    // pass paid for.
    culled.rounds += verdict.rounds;
    let keeper = match chosen {
        Ok(keeper) => keeper,
        Err(failure) => {
            culled.undecided += 1;
            verdict.failure = Some(failure);
            watcher.event(&Event::Group(verdict.clone()));
            culled.verdicts.push(verdict);
            return;
        }
    };
    if verdict.confidence < config.budget.floor {
        culled.low_confidence += 1;
    }
    verdict.keeper = Some(name(&row.frames[keeper]));
    mark_group(row, keeper, config, writer, watcher, culled, &mut verdict);
    watcher.event(&Event::Group(verdict.clone()));
    culled.verdicts.push(verdict);
}

/// Which frame of the group to keep: batches of at most one request's worth,
/// compared round by round until one frame survives.
///
/// Every frame of the group enters a request, in capture order, and the shakiest
/// confidence on the winner's path is what the verdict carries. A request that
/// cannot be built, answered or read fails the whole group: a smaller request in
/// its place would decide a group clef did not see all of.
fn choose(
    row: &GroupRow<'_>,
    config: &Config<'_>,
    decider: &dyn Decider,
    verdict: &mut Verdict,
) -> std::result::Result<usize, Failure> {
    // A request that can carry one frame cannot ask which of two to keep, and
    // comparing it against itself would never remove a frame from the field. The
    // measured budget is 20; the guard is what makes any other value a refusal
    // rather than an endless pass.
    if config.budget.max < 2 {
        return Err(Failure::of(&Error::new(
            ErrorKind::BadResponse,
            "a comparative request must carry at least two frames, and this one carries \
             fewer",
        )));
    }
    // Each contender carries the shakiest call it has survived so far. A batch's
    // losers take their confidence out of the field with them: what the verdict
    // depended on is the winners' path and nothing else.
    let mut contenders: Vec<(usize, f64)> =
        (0..row.frames.len()).map(|place| (place, 1.0)).collect();
    loop {
        let batches = keeper::chunk(&contenders, config.budget.max);
        let whole = batches.len() == 1;
        let mut winners = Vec::with_capacity(batches.len());
        for batch in &batches {
            // A batch of one has nothing to compare. It only exists when the cap
            // cannot split the field evenly, and it carries its own confidence
            // forward rather than spending a request on a question with one answer.
            if batch.len() == 1 {
                winners.push(batch[0]);
                continue;
            }
            let places: Vec<usize> = batch.iter().map(|(place, _)| *place).collect();
            verdict.rounds += 1;
            let choice = compare_batch(row, &places, config, decider)?;
            let (place, carried) = batch[choice.index()];
            winners.push((place, carried.min(choice.confidence)));
        }
        if whole {
            verdict.confidence = winners[0].1;
            return Ok(winners[0].0);
        }
        contenders = winners;
    }
}

/// One batch's comparative request: its previews at the measured edge, the
/// group's name and framing, and one answer naming the frame to keep.
fn compare_batch(
    row: &GroupRow<'_>,
    batch: &[usize],
    config: &Config<'_>,
    decider: &dyn Decider,
) -> std::result::Result<keeper::Choice, Failure> {
    let params = Params {
        max_edge: config.budget.edge,
        jpeg_quality: config.jpeg_quality,
    };
    let mut previews = Vec::with_capacity(batch.len());
    for &place in batch {
        previews.push(
            scoring::prepare(&row.frames[place].source, &params)
                .map_err(|error| Failure::of(&error))?,
        );
    }
    let comparison = Comparison {
        model: config.model.to_string(),
        state: State::for_frame(name(&row.frames[0]), config.shoot_context.clone()),
        previews,
    };
    let response = scoring::retrying(|| decider.compare(&comparison), &config.retry)
        .map_err(|error| Failure::of(&error))?;
    keeper::read(&response, batch.len()).map_err(|error| Failure::of(&error))
}

/// Write the verdict beside every frame of a decided group, keeper and
/// non-keeper alike.
fn mark_group(
    row: &GroupRow<'_>,
    keeper: usize,
    config: &Config<'_>,
    writer: &dyn Writer,
    watcher: &dyn Watcher,
    culled: &mut Culled,
    verdict: &mut Verdict,
) {
    let picked_at = crate::time::now_rfc3339();
    for (place, frame) in row.frames.iter().enumerate() {
        let existing = match sidecar::read(&frame.source.path) {
            Ok(existing) => existing,
            Err(error) => {
                culled.unwritten(frame, error.to_string(), watcher);
                continue;
            }
        };
        if existing
            .as_deref()
            .is_some_and(|document| !sidecar::can_extend(document))
        {
            culled.unwritten(
                frame,
                format!(
                    "{} has no rdf:Description this tool can write into",
                    sidecar::path_for(&frame.source.path).display()
                ),
                watcher,
            );
            continue;
        }
        if existing.as_deref().is_some_and(sidecar::is_foreign) && !config.allow_foreign {
            culled.unwritten(frame, FOREIGN, watcher);
            continue;
        }
        let keeper_here = place == keeper;
        let marking = Marking::of(keeper_here, existing.as_deref());
        let mark = Mark {
            label: marking.label(),
            keeper: keeper_here,
            group_start: row.frames[0]
                .captured
                .map_or_else(String::new, |captured| captured.to_string()),
            group_size: row.frames.len(),
            grouping_hash: config.grouping_hash.to_string(),
            pick_by: keeper::BY_COMPARISON.to_string(),
            pick_conf: verdict.confidence,
            picked_at: picked_at.clone(),
            rounds: verdict.rounds,
            retire_label: marking.retires_label(),
        };
        let document = match sidecar::mark_document(existing.as_deref(), &mark) {
            Ok(document) => document,
            Err(error) => {
                culled.unwritten(frame, error.to_string(), watcher);
                continue;
            }
        };
        let prior = Prior::earlier(existing.as_deref());
        if let Err(error) = writer.write(&frame.source.path, &document, prior) {
            culled.unwritten(frame, error.to_string(), watcher);
            continue;
        }
        match marking {
            Marking::Red => {
                culled.non_keepers += 1;
                culled.red += 1;
                verdict.red += 1;
            }
            Marking::Unmarked => {
                culled.non_keepers += 1;
                culled.unmarked += 1;
                verdict.unmarked += 1;
            }
            Marking::Retired => culled.retired += 1,
            Marking::StaleRed => culled.stale_reds += 1,
            Marking::Keeper => {}
        }
    }
}

/// A frame that is no longer in any group: the verdict a previous run wrote
/// about a comparison that no longer exists is retired, and so is the red.
///
/// Nothing is written unless the sidecar carries a verdict or a red this tool
/// wrote, so a folder of singletons costs one read per frame and no write.
fn retire_singleton(
    frame: &Planned,
    config: &Config<'_>,
    writer: &dyn Writer,
    watcher: &dyn Watcher,
    culled: &mut Culled,
) {
    let Ok(Some(existing)) = sidecar::read(&frame.source.path) else {
        return;
    };
    let ours_red = sidecar::existing_label(&existing).is_some_and(|label| label.is_ours_red());
    if !ours_red && !sidecar::has_verdict(&existing) {
        return;
    }
    if sidecar::is_foreign(&existing) && !config.allow_foreign {
        culled.unwritten(frame, FOREIGN, watcher);
        return;
    }
    let document = match sidecar::retire_label(&existing) {
        Ok(document) => document,
        Err(error) => {
            culled.unwritten(frame, error.to_string(), watcher);
            return;
        }
    };
    match writer.write(
        &frame.source.path,
        &document,
        Prior::earlier(Some(&existing)),
    ) {
        Ok(_) if ours_red => culled.retired += 1,
        Ok(_) => {}
        Err(error) => culled.unwritten(frame, error.to_string(), watcher),
    }
}

/// The one sentence a cull says when it will not write into another
/// application's sidecar: the same words wherever the refusal happens.
const FOREIGN: &str =
    "the sidecar belongs to another application; pass --allow-foreign to write into it";

/// What writing a frame's verdict does to its colour label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marking {
    /// The label was written, under this tool's name: the frame is red.
    Red,
    /// The label is the photographer's, so nothing was written and the frame
    /// stays out of the delete set.
    Unmarked,
    /// The keeper, whose red this tool wrote and this write removes.
    Retired,
    /// The keeper, still wearing a red this tool did not write.
    StaleRed,
    /// The keeper, whose mark is its rating.
    Keeper,
}

impl Marking {
    /// What this frame's verdict does to its label, from the two facts that
    /// decide it: whether the frame is the keeper, and whose colour is there.
    fn of(keeper: bool, existing: Option<&str>) -> Marking {
        let carried = existing.and_then(sidecar::existing_label);
        let ours = carried
            .as_ref()
            .is_some_and(|label| label.ours && label.value == sidecar::RED);
        let any_red = carried
            .as_ref()
            .is_some_and(|label| label.value == sidecar::RED);
        if keeper {
            return if ours {
                Marking::Retired
            } else if any_red {
                Marking::StaleRed
            } else {
                Marking::Keeper
            };
        }
        if sidecar::label_is_ours_to_write(existing) {
            Marking::Red
        } else {
            Marking::Unmarked
        }
    }

    /// The label this verdict writes, when it writes one.
    fn label(self) -> Option<&'static str> {
        match self {
            Marking::Red => Some(sidecar::RED),
            Marking::Unmarked | Marking::Retired | Marking::StaleRed | Marking::Keeper => None,
        }
    }

    /// Whether this verdict removes a red this tool wrote.
    fn retires_label(self) -> bool {
        matches!(self, Marking::Retired)
    }
}

/// The name a frame is reported under: its path inside the shoot folder, so a
/// cull reads the same names the plan's rows and the sidecars do.
fn name(frame: &Planned) -> String {
    frame.source.relative.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;

    use super::*;
    use crate::group::{Plan, Planned};
    use crate::ingest::{ScanCounts, SourceFile};
    use crate::keeper::Budget;
    use crate::measure::Measures;
    use crate::scoring::{Decider, Request, Response, RetryPolicy};

    /// A decider that fails the test if it is asked anything: a group that
    /// cannot render its previews must never send a request.
    struct Unasked {
        asked: Cell<usize>,
    }

    impl Decider for Unasked {
        fn decide(&self, _request: &Request<'_>) -> Result<Response> {
            self.asked.set(self.asked.get() + 1);
            panic!("a cull never rates a frame");
        }

        fn compare(&self, _comparison: &Comparison) -> Result<Response> {
            self.asked.set(self.asked.get() + 1);
            panic!("a group whose preview cannot be rendered is not compared");
        }
    }

    /// A decider whose server cannot answer: the pass must not ask it for a
    /// folder that has nothing to compare.
    struct NoServer;

    impl Decider for NoServer {
        fn decide(&self, _request: &Request<'_>) -> Result<Response> {
            panic!("a cull never rates a frame");
        }

        fn compare(&self, _comparison: &Comparison) -> Result<Response> {
            panic!("a folder with no group makes no comparative request");
        }

        fn preflight(&self) -> Result<()> {
            Err(Error::new(
                ErrorKind::ServerUnreachable,
                "nothing is listening on 127.0.0.1:8000",
            ))
        }
    }

    fn planned(name: &str, place: usize, size: usize) -> Planned {
        planned_in(1, name, place, size)
    }

    fn planned_in(group: usize, name: &str, place: usize, size: usize) -> Planned {
        Planned {
            source: SourceFile {
                path: PathBuf::from("/no-such-shoot").join(name),
                relative: PathBuf::from(name),
            },
            group,
            group_index: place,
            group_size: size,
            captured: None,
            measures: Measures::default(),
            failure: None,
        }
    }

    #[test]
    fn a_folder_with_no_group_is_culled_without_a_server() {
        let plan = Plan {
            frames: vec![
                planned_in(1, "DSC_0001.NEF", 1, 1),
                planned_in(2, "DSC_0002.NEF", 1, 1),
            ],
            groups: 2,
            hash: "0123456789abcdef".to_string(),
            counts: ScanCounts::default(),
            elapsed: std::time::Duration::ZERO,
            jobs: 1,
        };
        let config = Config {
            model: "clef-flash-oQ4e",
            shoot_context: None,
            budget: Budget::default(),
            jpeg_quality: Params::default().jpeg_quality,
            grouping_hash: &plan.hash,
            allow_foreign: false,
            retry: RetryPolicy::without_waiting(),
        };

        let culled = run(&plan, &config, &NoServer, &sidecar::Filesystem, &Silent)
            .expect("a folder of singletons needs no server");

        assert_eq!(culled.groups, 0);
        assert_eq!(culled.undecided, 0);
    }

    #[test]
    fn a_request_that_can_carry_one_frame_refuses_the_group_instead_of_looping() {
        let plan = Plan {
            frames: vec![planned("DSC_0001.NEF", 1, 2), planned("DSC_0002.NEF", 2, 2)],
            groups: 1,
            hash: "0123456789abcdef".to_string(),
            counts: ScanCounts::default(),
            elapsed: std::time::Duration::ZERO,
            jobs: 1,
        };
        let config = Config {
            model: "clef-flash-oQ4e",
            shoot_context: None,
            budget: Budget {
                max: 1,
                ..Budget::default()
            },
            jpeg_quality: Params::default().jpeg_quality,
            grouping_hash: &plan.hash,
            allow_foreign: false,
            retry: RetryPolicy::without_waiting(),
        };
        let decider = Unasked {
            asked: Cell::new(0),
        };

        let culled = run(&plan, &config, &decider, &sidecar::Filesystem, &Silent)
            .expect("a misconfigured budget is a group's failure, not the run's");

        assert_eq!(culled.undecided, 1);
        assert_eq!(decider.asked.get(), 0);
        let failure = culled.verdicts[0]
            .failure
            .as_ref()
            .expect("the group says why it has no keeper");
        assert!(
            failure.message.contains("at least two frames"),
            "{}",
            failure.message
        );
    }

    #[test]
    fn an_unrenderable_member_leaves_the_group_undecided_without_a_request() {
        let plan = Plan {
            frames: vec![planned("DSC_0001.NEF", 1, 2), planned("DSC_0002.NEF", 2, 2)],
            groups: 1,
            hash: "0123456789abcdef".to_string(),
            counts: ScanCounts::default(),
            elapsed: std::time::Duration::ZERO,
            jobs: 1,
        };
        let config = Config {
            model: "clef-flash-oQ4e",
            shoot_context: None,
            budget: Budget::default(),
            jpeg_quality: Params::default().jpeg_quality,
            grouping_hash: &plan.hash,
            allow_foreign: false,
            retry: RetryPolicy::without_waiting(),
        };
        let decider = Unasked {
            asked: Cell::new(0),
        };

        let culled = run(&plan, &config, &decider, &sidecar::Filesystem, &Silent)
            .expect("a plan with one unreadable frame is still a cull");

        assert_eq!(decider.asked.get(), 0, "no smaller request is sent instead");
        assert_eq!(culled.undecided, 1);
        assert_eq!(culled.non_keepers, 0, "nothing is marked on no answer");
        assert_eq!(
            culled.verdicts[0]
                .failure
                .as_ref()
                .map(|failure| failure.kind),
            Some(ErrorKind::Unreadable),
            "the group says why it has no keeper"
        );
    }
}
