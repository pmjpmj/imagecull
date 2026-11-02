//! The cull: every group of a shoot, the keeper clef chose, and the green or
//! yellow/red mark on everyone else.
//!
//! `imagecull groups` finds the groups and measures them; this module is what
//! turns that plan into a decision. One comparative request per round asks which
//! frame of a group to keep, the answer names one, and the group is marked with
//! that answer: the keeper green, every other frame red — or yellow when the
//! comparison was below the confidence floor, because a near-tie is worth a look
//! before it is deleted. A group of one is asked nothing and marked nothing — but
//! a mark this tool wrote on a frame that is no longer in a group is retired,
//! because the group it lost does not exist any more.
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
//! The rules of ADR 0007 hold: a photographer's colour is never overwritten, a
//! label without our `labeledBy` proof is never removed, and a group nothing
//! could decide marks nothing. ADR 0008 adds the one exception: a mark this tool
//! wrote is rewritten when a later run changes the verdict, because the cull is
//! repeatable and a mark that says the keeper lost is the one thing it must not
//! leave.
//!
//! # Nothing here knows a terminal or a command line
//!
//! The pass reports what it did through a [`Watcher`] and returns counts; the
//! command renders them. The model call goes through
//! [`Decider::compare`](crate::scoring::Decider::compare), so a whole cull runs
//! in a test with no server.

use std::sync::Mutex;
use std::sync::mpsc;
use std::thread;

use crate::error::{Error, ErrorKind, Result};
use crate::group::{self, GroupRow, Plan, Planned};
use crate::keeper::{self, Budget, Comparison};
use crate::preprocess::Params;
use crate::scoring::{self, Decider, State};
use crate::sidecar::{self, Failure, Mark, Prior, Writer};

/// What one cull pass did, group by group and in total.
///
/// Every count counts frames except `groups`, `rounds`, `low_confidence` and
/// `undecided`, which count groups or requests. The counts that describe
/// non-keepers — `non_keepers`, `red`, `yellow` and `unmarked` — count the same
/// population, so `non_keepers` is `red` plus `yellow` plus `unmarked` and the
/// summary's arithmetic adds up. `green` and `keeper_unmarked` count the keepers
/// of those same groups.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Culled {
    /// How many groups of two or more frames were judged.
    pub groups: usize,
    /// How many non-keepers carry this run's verdict.
    pub non_keepers: usize,
    /// How many of them are red: they lost to a verdict at or above the floor.
    pub red: usize,
    /// How many of them are yellow: they lost to a verdict below the floor. Every
    /// yellow is a near-tie the photographer may want to review before deleting.
    pub yellow: usize,
    /// How many keepers are green.
    pub green: usize,
    /// How many were left unmarked because the label on the frame is not ours to
    /// write.
    pub unmarked: usize,
    /// How many keepers were left unmarked for the same reason.
    pub keeper_unmarked: usize,
    /// How many frames that lost their group had a mark this tool wrote removed.
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
    /// How many non-keepers this group marked red.
    pub red: usize,
    /// How many non-keepers it marked yellow, because the verdict was below the
    /// confidence floor.
    pub yellow: usize,
    /// How many keepers it marked green; 1 for a decided group whose colour was
    /// ours to write, 0 when the photographer's label was left alone.
    pub green: usize,
    /// How many it left to the photographer.
    pub unmarked: usize,
    /// How many keepers it left to the photographer, for the same reason.
    pub keeper_unmarked: usize,
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
    /// The comparison's shape: at most how many frames one request carries, and
    /// the confidence under which a verdict is reported as shaky.
    pub budget: Budget,
    /// The payload shape of one image in a request: its longest edge and its
    /// JPEG quality. The run's own — `[group] max_edge` / `[group]
    /// jpeg_quality`, `groups --max-edge` / `--jpeg-quality` — defaulting to
    /// [`keeper::EDGE`] and [`Params::DEFAULT_JPEG_QUALITY`].
    pub preview: Params,
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
        ///
        /// `None` when the cull starts before the folder has been fully grouped,
        /// which is the streaming pass: a group is judged as soon as it is final,
        /// so the total is not known when the pass begins.
        groups: Option<usize>,
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
    /// Add one group's outcome to a pass's totals.
    ///
    /// The streaming pass judges groups on several threads, so each worker
    /// returns the counts of the groups it took and the pass adds them here.
    /// Verdicts keep the order they were added; the streaming pass sorts them by
    /// group number before it reports.
    pub fn merge(&mut self, other: Culled) {
        self.groups += other.groups;
        self.non_keepers += other.non_keepers;
        self.red += other.red;
        self.yellow += other.yellow;
        self.green += other.green;
        self.unmarked += other.unmarked;
        self.keeper_unmarked += other.keeper_unmarked;
        self.retired += other.retired;
        self.stale_reds += other.stale_reds;
        self.low_confidence += other.low_confidence;
        self.undecided += other.undecided;
        self.unwritten += other.unwritten;
        self.rounds += other.rounds;
        self.verdicts.extend(other.verdicts);
    }

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
    watcher.event(&Event::Started {
        groups: Some(groups),
    });
    // A folder with no group asks no comparative question, so it needs no
    // server: the preflight is about the requests the pass will make, and a
    // shoot of singletons makes none.
    if groups > 0 {
        decider.preflight()?;
    }
    let mut culled = Culled::default();
    for row in &rows {
        culled.merge(judge(row, config, decider, writer, watcher));
    }
    Ok(culled)
}

/// Judge one group and write what it decided.
///
/// A group of one asks no comparative question and is retired instead; a group
/// of two or more is compared and marked. The caller owns the preflight, so a
/// pass that judges groups in parallel can ask the server once rather than once
/// per worker.
pub fn judge(
    row: &GroupRow<'_>,
    config: &Config<'_>,
    decider: &dyn Decider,
    writer: &dyn Writer,
    watcher: &dyn Watcher,
) -> Culled {
    let mut culled = Culled::default();
    match row.frames {
        [only] => retire_singleton(only, config, writer, watcher, &mut culled),
        _ => judge_group(row, config, decider, writer, watcher, &mut culled),
    }
    culled
}

/// How many groups a streamed cull judges at once when the caller does not say.
///
/// Four is a compromise: enough groups in flight to overlap preview rendering
/// with a server's latency, few enough that the previews of several large groups
/// are not all held in memory at once.
pub const DEFAULT_JOBS: usize = 4;

/// What a cull that ran alongside the grouping pass leaves behind.
#[derive(Debug)]
pub struct Streamed {
    /// The plan the grouping pass built, whole.
    pub plan: Plan,
    /// What the groups that could be judged decided.
    pub culled: Culled,
    /// The error that stopped the cull, when one did.
    ///
    /// A fatal preflight is the case this exists for: the grouping pass still
    /// finishes, so the caller has a plan and can write the measures CSV, but no
    /// verdict was written and the run's exit is fatal.
    pub fatal: Option<Error>,
}

/// Judge groups as a grouping pass finds them, `jobs` at a time, and wait for
/// every one before returning.
///
/// `grouping` is the pass that finds the groups — [`group::stream_with`] in
/// production, a script in a test — and it runs on this thread while the workers
/// judge what it hands over. The first group of two or more triggers the one
/// preflight, before any worker is given a group: a server that cannot answer
/// costs seconds, and nothing is marked first. A preflight that fails does not
/// stop the grouping pass, so the caller still gets a plan to write; it stops
/// the cull, and comes back in [`Streamed::fatal`].
///
/// The requests of different groups may be in flight at once. Groups share no
/// frame, so their sidecar writes never meet; the watcher is `Sync`, and the
/// terminal locks each line as it writes it, so two events cannot split each
/// other. A group's own frames are still written one at a time, in capture
/// order.
pub fn run_streaming<F>(
    config: &Config<'_>,
    decider: &(dyn Decider + Sync),
    writer: &(dyn Writer + Sync),
    watcher: &(dyn Watcher + Sync),
    jobs: usize,
    grouping: F,
) -> Result<Streamed>
where
    F: FnOnce(&mut dyn group::Sink) -> Result<Plan>,
{
    let jobs = jobs.max(1);
    let (sender, receiver) = mpsc::channel::<group::Found>();
    let receiver = Mutex::new(receiver);
    let results = Mutex::new(Culled::default());
    let receiver = &receiver;
    let results = &results;
    thread::scope(|scope| {
        let mut handles = Vec::with_capacity(jobs);
        for _ in 0..jobs {
            handles.push(scope.spawn(move || {
                loop {
                    let found = match receiver.lock().expect("a cull worker's queue").recv() {
                        Ok(found) => found,
                        Err(_) => break,
                    };
                    let row = found.row();
                    let one = judge(&row, config, decider, writer, watcher);
                    results.lock().expect("the cull's totals").merge(one);
                }
            }));
        }

        let mut pool = Pool::new(sender, decider, watcher);
        let plan = grouping(&mut pool);
        if plan.is_ok() {
            pool.finish();
        }
        let fatal = pool.fatal.take();
        // Dropping the sender is what ends the workers once the queue is dry.
        drop(pool);
        for handle in handles {
            handle
                .join()
                .expect("a cull worker does not panic before the pass ends");
        }

        let mut culled =
            std::mem::take(&mut *results.lock().expect("the cull's totals are not poisoned"));
        // Workers finish in whatever order the model answers; the report reads
        // groups by number, and this is what makes a second run's report byte for
        // byte the same.
        culled.verdicts.sort_by_key(|verdict| verdict.number);
        plan.map(|plan| Streamed {
            plan,
            culled,
            fatal,
        })
    })
}

/// The sink a streamed cull gives the grouping pass: it starts the one preflight,
/// hands every group to a worker, and holds the first groups back until the
/// folder has shown it has a group worth asking the model about.
///
/// The hold-back is the preflight's ordering rule. A folder of singletons needs no
/// server, and a folder with one group of two needs the server before its frames'
/// sidecars are touched — including the singletons' retirements, which is why they
/// wait for the first multi-frame group rather than being judged as they arrive.
struct Pool<'a> {
    sender: mpsc::Sender<group::Found>,
    decider: &'a (dyn Decider + Sync),
    watcher: &'a (dyn Watcher + Sync),
    /// Whether the pass has announced itself: the start line says a count only
    /// when the folder is already fully grouped, which streaming cannot know.
    announced: bool,
    /// Whether the preflight passed and groups may be dispatched.
    started: bool,
    /// Singletons seen before the first group of two or more.
    deferred: Vec<group::Found>,
    /// The error that stopped the cull.
    fatal: Option<Error>,
}

impl<'a> Pool<'a> {
    fn new(
        sender: mpsc::Sender<group::Found>,
        decider: &'a (dyn Decider + Sync),
        watcher: &'a (dyn Watcher + Sync),
    ) -> Pool<'a> {
        Pool {
            sender,
            decider,
            watcher,
            announced: false,
            started: false,
            deferred: Vec::new(),
            fatal: None,
        }
    }

    /// Hand one group to a worker. A worker that panicked drops the receiver;
    /// the join reports the panic, and the send's failure is that run's problem
    /// rather than this line's.
    fn dispatch(&self, found: group::Found) {
        let _ = self.sender.send(found);
    }

    /// The grouping pass is over: release what waited for the preflight, or say
    /// that there was no group to judge at all.
    fn finish(&mut self) {
        if self.fatal.is_some() {
            return;
        }
        if !self.announced {
            self.watcher.event(&Event::Started { groups: Some(0) });
        }
        for found in std::mem::take(&mut self.deferred) {
            self.dispatch(found);
        }
    }
}

impl group::Sink for Pool<'_> {
    fn group(&mut self, found: group::Found) -> Result<()> {
        if self.fatal.is_some() {
            // The cull cannot be finished, but the grouping pass still has a plan
            // to build and a CSV to write.
            return Ok(());
        }
        if self.started {
            self.dispatch(found);
            return Ok(());
        }
        if found.size() > 1 {
            self.announced = true;
            self.watcher.event(&Event::Started { groups: None });
            if let Err(error) = self.decider.preflight() {
                self.fatal = Some(error);
                return Ok(());
            }
            self.started = true;
            for deferred in std::mem::take(&mut self.deferred) {
                self.dispatch(deferred);
            }
            self.dispatch(found);
        } else {
            self.deferred.push(found);
        }
        Ok(())
    }
}

/// One group of two or more: compare its frames in rounds, then mark every
/// member of the group with what that decided.
fn judge_group(
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
        yellow: 0,
        green: 0,
        unmarked: 0,
        keeper_unmarked: 0,
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
    let low_confidence = verdict.confidence < config.budget.floor;
    if low_confidence {
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

/// One batch's comparative request: its previews at the run's comparison edge,
/// the group's name and framing, and one answer naming the frame to keep.
fn compare_batch(
    row: &GroupRow<'_>,
    batch: &[usize],
    config: &Config<'_>,
    decider: &dyn Decider,
) -> std::result::Result<keeper::Choice, Failure> {
    let mut previews = Vec::with_capacity(batch.len());
    for &place in batch {
        previews.push(
            scoring::prepare(&row.frames[place].source, &config.preview)
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
/// non-keeper alike. The keeper is green, and a non-keeper of a low-confidence
/// group is yellow rather than red: the comparison was a near-tie, and a frame
/// that may be the better picture should not be filtered out as if it were sure.
fn mark_group(
    row: &GroupRow<'_>,
    keeper: usize,
    config: &Config<'_>,
    writer: &dyn Writer,
    watcher: &dyn Watcher,
    culled: &mut Culled,
    verdict: &mut Verdict,
) {
    let low_confidence = verdict.confidence < config.budget.floor;
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
        let marking = Marking::of(keeper_here, low_confidence, existing.as_deref());
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
            // A decided group rewrites its members' colours; nothing is removed
            // outright. Retirement is the singleton path's business.
            retire_label: false,
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
            Marking::Yellow => {
                culled.non_keepers += 1;
                culled.yellow += 1;
                verdict.yellow += 1;
            }
            Marking::Green => {
                culled.green += 1;
                verdict.green += 1;
            }
            Marking::Unmarked => {
                culled.non_keepers += 1;
                culled.unmarked += 1;
                verdict.unmarked += 1;
            }
            Marking::KeeperUnmarked => {
                culled.keeper_unmarked += 1;
                verdict.keeper_unmarked += 1;
            }
            Marking::StaleRed => culled.stale_reds += 1,
        }
    }
}

/// A frame that is no longer in any group: the verdict a previous run wrote
/// about a comparison that no longer exists is retired, and so is the mark —
/// red, yellow or green — this tool wrote.
///
/// Nothing is written unless the sidecar carries a verdict or a mark this tool
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
    let ours_mark = sidecar::existing_label(&existing).is_some_and(|label| label.is_ours());
    if !ours_mark && !sidecar::has_verdict(&existing) {
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
        Ok(_) if ours_mark => culled.retired += 1,
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
    /// A non-keeper that lost to a verdict the comparison was sure of: red.
    Red,
    /// A non-keeper that lost to a verdict below the confidence floor: yellow,
    /// so the photographer reviews the near-tie before deleting.
    Yellow,
    /// The keeper: green, the answer to the comparison.
    Green,
    /// A non-keeper whose colour is the photographer's, so nothing was written
    /// and the frame stays out of the delete set.
    Unmarked,
    /// The keeper, still wearing a red this tool did not write.
    StaleRed,
    /// The keeper, wearing a colour this tool did not write, so it cannot be
    /// made green.
    KeeperUnmarked,
}

impl Marking {
    /// What this frame's verdict does to its label, from the facts that decide
    /// it: whether the frame is the keeper, whether the verdict was below the
    /// floor, and whose colour is there.
    fn of(keeper: bool, low_confidence: bool, existing: Option<&str>) -> Marking {
        let carried = existing.and_then(sidecar::existing_label);
        let ours_to_write = sidecar::label_is_ours_to_write(existing);
        if keeper {
            return if ours_to_write {
                Marking::Green
            } else if carried
                .as_ref()
                .is_some_and(|label| label.value == sidecar::RED)
            {
                Marking::StaleRed
            } else {
                Marking::KeeperUnmarked
            };
        }
        if !ours_to_write {
            return Marking::Unmarked;
        }
        if low_confidence {
            Marking::Yellow
        } else {
            Marking::Red
        }
    }

    /// The label this verdict writes, when it writes one.
    fn label(self) -> Option<&'static str> {
        match self {
            Marking::Red => Some(sidecar::RED),
            Marking::Yellow => Some(sidecar::YELLOW),
            Marking::Green => Some(sidecar::GREEN),
            Marking::Unmarked | Marking::StaleRed | Marking::KeeperUnmarked => None,
        }
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
            preview: Params::default(),
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
    fn a_streamed_folder_of_singletons_asks_the_server_nothing() {
        let hash = "0123456789abcdef".to_string();
        let config = Config {
            model: "clef-flash-oQ4e",
            shoot_context: None,
            budget: Budget::default(),
            preview: Params::default(),
            grouping_hash: &hash,
            allow_foreign: false,
            retry: RetryPolicy::without_waiting(),
        };

        let streamed = run_streaming(
            &config,
            &NoServer,
            &sidecar::Filesystem,
            &Silent,
            2,
            |sink| {
                sink.group(group::Found {
                    number: 1,
                    frames: vec![planned_in(1, "DSC_0001.NEF", 1, 1)],
                })?;
                sink.group(group::Found {
                    number: 2,
                    frames: vec![planned_in(2, "DSC_0002.NEF", 1, 1)],
                })?;
                Ok(Plan {
                    frames: Vec::new(),
                    groups: 2,
                    hash: hash.clone(),
                    counts: ScanCounts::default(),
                    elapsed: std::time::Duration::ZERO,
                    jobs: 1,
                })
            },
        )
        .expect("a folder of singletons needs no server, streamed or not");

        assert!(streamed.fatal.is_none());
        assert_eq!(streamed.culled.groups, 0, "nothing was judged");
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
            preview: Params::default(),
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
            preview: Params::default(),
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
