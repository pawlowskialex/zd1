//! The update plan (docs/web.md, Updates): the cheapest sequence of routes that brings a speaker to a release, from
//! what the speaker and the release say. Pure: the front-ends gather the [`Facts`], run the [`Step`]s with the flows
//! they already have (`update`, `install`, `flash`, the page's BL2 upload), and say what is left.
//!
//! A route changes the speaker's [`State`]: which parts are behind (the module's boot image, its system, the main
//! board's firmware), what the main board can do, whether an MCU app is staged on the module. Its cost is its time
//! plus what it asks of a person (a hold of ON/OFF, unplugging the speaker); the plan is the shortest path to the
//! state with the fewest parts behind. The routes, their preconditions and effects are the docs' (docs/module/
//! updater.md, The flows); a part a route brings as the boot image's rescue is not copied again when a slot already
//! holds those bytes (module/os/apps/zd1-init, self_install).

use std::collections::{BinaryHeap, HashMap};

/// Which speaker (the host link's `Model`; a firmware older than that field: told by its refusal of `Dfu`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Model {
    /// the Zeppelin Air: B&W's DFU bootloader (our preloader behind it), the relay
    Zd1,
    /// the Zeppelin Air with Lightning: B&W's "Net HBL", no USB DFU; its MCU firmware from the module
    Zd1l,
}

/// How the front-end reaches the speaker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Link {
    Usb,
    Net,
}

/// What the module runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Module {
    /// our OS from a system slot
    Os,
    /// our OS as the boot image's rescue: no system in a slot
    Rescue,
    /// BL2, with no boot image to start (a boot image's write that stopped)
    Bl2,
    /// B&W's software (an install that stopped after the main board)
    Stock,
}

/// What the release offers (images/index.json).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Has {
    /// the module's system (USB)
    pub system: bool,
    /// ... signed for the network
    pub signed: bool,
    /// the Zeppelin Air's boot image (`os-V.bcd`)
    pub bcd: bool,
    /// the main board's firmware (`zd1-V.hex`, DFU)
    pub mcu: bool,
    /// the Lightning model's `.fw` (both chips, BL2's web updater)
    pub fw: bool,
}

/// What a plan starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Facts {
    pub model: Model,
    pub link: Link,
    pub module: Module,
    /// behind the release: the module's boot image, its system, the main board's firmware
    pub boot: bool,
    pub system: bool,
    pub mcu: bool,
    /// the rescue that runs (`Module::Rescue`) is the release's system
    pub rescue_is_release: bool,
    /// the module on wired Ethernet with an address the front-end knows (BL2's web updater has no Wi-Fi)
    pub ethernet: bool,
    /// the main board's firmware has `Allow` (it takes an app from the module after a hold) and `Bl2Updater`
    pub allow: bool,
    pub updater: bool,
    /// an MCU app is staged on the module: a system from before 0.5.1 keeps it, a later one until the main board runs
    /// it. A rescue from before 0.5.1 serves it to B&W's bootloader instead of starting BL2 (docs/mcu/bootloader-hbl.md);
    /// which rescue a boot image holds the speaker doesn't say, so any boot image this plan didn't write counts as such
    pub staged: bool,
    pub has: Has,
}

/// A way to bring parts of the speaker to the release.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Route {
    /// the system over USB into the slot that doesn't run, started once (`update::system`)
    SystemUsb,
    /// the system over USB, a hold (`Allow`), then its slot started once: at its ready the main board takes the app
    /// the system carries from the module (B&W's bootloader on a ZD1L, our preloader on a ZD1)
    SystemFollow,
    /// a hold (`Allow`), then a module restart: the main board takes the app the running system carries
    Follow,
    /// the signed system over the network, allowed by a hold, which also allows the main board the app it carries
    SystemNet,
    /// the rescue into a slot (`install::system_from_rescue`)
    Rescue,
    /// the main board's firmware over DFU, a backup first (`flash`)
    Dfu,
    /// the boot image through the DFU bootloader's relay; its rescue then installs itself (`install::boot_image`)
    Relay,
    /// the boot image and the main board's firmware in one visit to the bootloader, the system from the rescue
    /// (`install::install`)
    Install,
    /// the boot image through BL2's XModem, the module in BL2 with nothing to start (`install::boot_image_in_bl2`)
    Bl2Xmodem,
    /// a hold (`Bl2Updater`), the main board into B&W's bootloader, the module into BL2's web updater on its
    /// Ethernet, the release's `.fw` to it: both chips, the system from the rescue
    Updater,
    /// the same by hand: the speaker unplugged, ON/OFF held while plugging it in, the `.fw` by address
    KeyHold,
}

impl Route {
    pub const ALL: [Route; 11] = [
        Route::SystemUsb,
        Route::SystemFollow,
        Route::Follow,
        Route::SystemNet,
        Route::Rescue,
        Route::Dfu,
        Route::Relay,
        Route::Install,
        Route::Bl2Xmodem,
        Route::Updater,
        Route::KeyHold,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Route::SystemUsb => "system-usb",
            Route::SystemFollow => "system-follow",
            Route::Follow => "follow",
            Route::SystemNet => "system-net",
            Route::Rescue => "rescue",
            Route::Dfu => "dfu",
            Route::Relay => "relay",
            Route::Install => "install",
            Route::Bl2Xmodem => "bl2-xmodem",
            Route::Updater => "updater",
            Route::KeyHold => "key-hold",
        }
    }

    /// Its time, s (docs/module/updater.md: measured where it ran on a unit, else the emulator's or a guess).
    pub fn secs(self) -> u32 {
        match self {
            Route::SystemUsb => 120,
            Route::SystemFollow => 200,
            Route::Follow => 100,
            Route::SystemNet => 70,
            Route::Rescue => 70,
            Route::Dfu => 40,
            Route::Relay => 430,
            Route::Install => 450,
            Route::Bl2Xmodem => 1000,
            Route::Updater => 600,
            Route::KeyHold => 600,
        }
    }

    /// Holds of ON/OFF it asks for.
    pub fn holds(self) -> u32 {
        match self {
            Route::SystemFollow | Route::Follow | Route::SystemNet | Route::Updater | Route::KeyHold => 1,
            _ => 0,
        }
    }

    /// The speaker unplugged by hand (the key-hold).
    pub fn unplug(self) -> bool {
        self == Route::KeyHold
    }

    /// A person does it all (the front-end only says how).
    pub fn by_hand(self) -> bool {
        self == Route::KeyHold
    }

    /// What it costs: its time, a hold as 30 s, unplugging as 15 minutes (a person walking to the speaker and back).
    pub fn cost(self) -> u32 {
        self.secs() + 30 * self.holds() + if self.unplug() { 900 } else { 0 }
    }
}

/// The speaker as a plan sees it: [`Facts`]' parts and abilities, changed by each route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct State {
    pub boot: bool,
    pub system: bool,
    pub mcu: bool,
    pub module: u8,
    pub allow: bool,
    pub updater: bool,
    pub staged: bool,
    /// the boot image's rescue is from before 0.5.1 (any this plan didn't write)
    pub old_rescue: bool,
    pub rescue_is_release: bool,
}

const OS: u8 = 0;
const RESCUE: u8 = 1;
const BL2: u8 = 2;
const STOCK: u8 = 3;

impl State {
    pub fn from(f: &Facts) -> State {
        State {
            boot: f.boot,
            system: f.system,
            mcu: f.mcu,
            module: match f.module {
                Module::Os => OS,
                Module::Rescue => RESCUE,
                Module::Bl2 => BL2,
                Module::Stock => STOCK,
            },
            allow: f.allow,
            updater: f.updater,
            staged: f.staged,
            old_rescue: true,
            rescue_is_release: f.rescue_is_release,
        }
    }

    /// Parts behind.
    pub fn left(&self) -> u32 {
        self.boot as u32 + self.system as u32 + self.mcu as u32
    }

    fn ours(&self) -> bool {
        self.module == OS || self.module == RESCUE
    }

    /// The release on the main board: it has every request, and runs what the release's system carries.
    fn mcu_now(&mut self) {
        self.mcu = false;
        self.allow = true;
        self.updater = true;
    }

    /// The release's system in a slot: it stages the app it carries while the main board runs another.
    fn system_now(&mut self) {
        self.system = false;
        self.module = OS;
        self.staged = self.mcu;
    }

    /// The release's boot image: its rescue is the release's system, and follows the asks (0.5.1 on).
    fn boot_now(&mut self) {
        self.boot = false;
        self.old_rescue = false;
        self.system_now();
    }
}

/// Whether `r` can run on `s` (with `f`'s fixed facts), and if it does anything.
pub fn applies(r: Route, f: &Facts, s: &State) -> bool {
    let usb = f.link == Link::Usb;
    let zd1 = f.model == Model::Zd1;
    match r {
        Route::SystemUsb => usb && s.ours() && f.has.system && s.system,
        Route::SystemFollow => usb && s.ours() && f.has.system && s.system && s.mcu && s.allow,
        Route::Follow => usb && s.module == OS && !s.system && s.mcu && s.allow,
        Route::SystemNet => !usb && s.module == OS && f.has.signed && s.system,
        Route::Rescue => usb && s.module == RESCUE && s.rescue_is_release && s.system,
        Route::Dfu => zd1 && usb && f.has.mcu && s.mcu,
        Route::Relay => zd1 && usb && f.has.bcd && s.boot && s.module != STOCK,
        Route::Install => zd1 && usb && f.has.bcd && f.has.mcu && (s.boot || s.module == STOCK),
        Route::Bl2Xmodem => zd1 && usb && f.has.bcd && s.module == BL2,
        Route::Updater => {
            !zd1 && f.has.fw
                && f.ethernet
                && s.ours()
                && s.updater
                && !(s.staged && s.old_rescue)
                && (s.boot || s.system || s.mcu)
        }
        Route::KeyHold => !zd1 && f.has.fw && (s.boot || s.system || s.mcu || s.module != OS),
    }
}

/// `s` after `r` (which [`applies`]).
pub fn apply(r: Route, s: &State) -> State {
    let mut n = *s;
    match r {
        Route::SystemUsb => n.system_now(),
        Route::SystemFollow => {
            n.mcu_now();
            n.system_now();
        }
        Route::Follow => {
            n.mcu_now();
            n.staged = false;
        }
        Route::SystemNet => {
            // the update's hold also allows the main board the app the system carries
            if n.mcu && n.allow {
                n.mcu_now();
            }
            n.system_now();
        }
        Route::Rescue => {
            n.system_now();
            n.rescue_is_release = false;
        }
        Route::Dfu => {
            n.mcu_now();
            n.staged = n.staged && n.system;
        }
        Route::Relay | Route::Bl2Xmodem => n.boot_now(),
        Route::Install | Route::Updater | Route::KeyHold => {
            n.mcu_now();
            n.boot_now();
        }
    }
    n
}

/// One route of a plan, and the parts it brings to the release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub route: Route,
    pub boot: bool,
    pub system: bool,
    pub mcu: bool,
}

/// The cheapest plan: its steps, what it costs, and the parts no route brings from here (`left_*`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub steps: Vec<Step>,
    pub secs: u32,
    pub cost: u32,
    pub left_boot: bool,
    pub left_system: bool,
    pub left_mcu: bool,
}

/// The plan for `f`: of the states the routes reach, one with the fewest parts behind, by the cheapest path. Routes a
/// person does all of (`Route::by_hand`) only with `by_hand`: the front-end runs the plan without them, and says what
/// the plan with them would do about the parts left.
pub fn plan(f: &Facts, by_hand: bool) -> Plan {
    let start = State::from(f);
    let mut best: HashMap<State, (u32, Option<(State, Route)>)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    best.insert(start, (0, None));
    heap.push(std::cmp::Reverse((0u32, start)));
    while let Some(std::cmp::Reverse((c, s))) = heap.pop() {
        if best.get(&s).is_some_and(|&(b, _)| b < c) {
            continue;
        }
        for r in Route::ALL {
            if !applies(r, f, &s) || (r.by_hand() && !by_hand) {
                continue;
            }
            let n = apply(r, &s);
            let nc = c + r.cost();
            if best.get(&n).is_none_or(|&(b, _)| nc < b) {
                best.insert(n, (nc, Some((s, r))));
                heap.push(std::cmp::Reverse((nc, n)));
            }
        }
    }
    // the goal: fewest parts behind, then cheapest, then the fewest steps (a tie between equal routes stays stable)
    let steps_to = |s: &State| {
        let mut n = 0;
        let mut at = *s;
        while let Some((p, _)) = best[&at].1 {
            n += 1;
            at = p;
        }
        n
    };
    let goal = *best
        .iter()
        .min_by_key(|(s, (c, _))| (s.left(), *c, steps_to(s), **s))
        .map(|(s, _)| s)
        .expect("the start is in it");
    let mut steps = Vec::new();
    let mut at = goal;
    while let Some((p, r)) = best[&at].1 {
        steps.push(Step { route: r, boot: p.boot && !at.boot, system: p.system && !at.system, mcu: p.mcu && !at.mcu });
        at = p;
    }
    steps.reverse();
    Plan {
        secs: steps.iter().map(|s| s.route.secs()).sum(),
        cost: best[&goal].0,
        steps,
        left_boot: goal.boot,
        left_system: goal.system,
        left_mcu: goal.mcu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_HAS: Has = Has { system: true, signed: true, bcd: true, mcu: true, fw: true };

    fn facts(model: Model, link: Link) -> Facts {
        Facts {
            model,
            link,
            module: Module::Os,
            boot: true,
            system: true,
            mcu: true,
            rescue_is_release: false,
            ethernet: true,
            allow: true,
            updater: true,
            staged: false,
            has: ALL_HAS,
        }
    }

    fn plan_auto(f: &Facts) -> Plan {
        plan(f, false)
    }

    fn routes(p: &Plan) -> Vec<Route> {
        p.steps.iter().map(|s| s.route).collect()
    }

    /// Every combination of the facts (the release's images all there, or each one missing).
    fn every() -> Vec<Facts> {
        let mut out = Vec::new();
        for model in [Model::Zd1, Model::Zd1l] {
            for link in [Link::Usb, Link::Net] {
                for module in [Module::Os, Module::Rescue, Module::Bl2, Module::Stock] {
                    for bits in 0u32..256 {
                        let bit = |k: u32| bits >> k & 1 == 1;
                        for missing in 0..6 {
                            let mut has = ALL_HAS;
                            match missing {
                                1 => has.system = false,
                                2 => has.signed = false,
                                3 => has.bcd = false,
                                4 => has.mcu = false,
                                5 => has.fw = false,
                                _ => {}
                            }
                            out.push(Facts {
                                model,
                                link,
                                module,
                                boot: bit(0),
                                system: bit(1),
                                mcu: bit(2),
                                rescue_is_release: bit(3),
                                ethernet: bit(4),
                                allow: bit(5),
                                updater: bit(6),
                                staged: bit(7),
                                has,
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// The best (parts left, cost) of every sequence of up to `depth` routes, by brute force.
    fn brute(f: &Facts, by_hand: bool, s: State, depth: u32, cost: u32, best: &mut (u32, u32)) {
        let here = (s.left(), cost);
        if here < *best {
            *best = here;
        }
        if depth == 0 {
            return;
        }
        for r in Route::ALL {
            if applies(r, f, &s) && (by_hand || !r.by_hand()) {
                brute(f, by_hand, apply(r, &s), depth - 1, cost + r.cost(), best);
            }
        }
    }

    #[test]
    fn as_good_as_every_sequence() {
        for f in every() {
            for by_hand in [false, true] {
                let p = plan(&f, by_hand);
                let mut best = (u32::MAX, u32::MAX);
                brute(&f, by_hand, State::from(&f), 4, 0, &mut best);
                let left = p.left_boot as u32 + p.left_system as u32 + p.left_mcu as u32;
                assert_eq!((left, p.cost), best, "{f:?} by hand {by_hand}: {:?}", routes(&p));
            }
        }
    }

    #[test]
    fn every_step_brings_something_and_runs() {
        for (f, by_hand) in every().into_iter().flat_map(|f| [(f, false), (f, true)]) {
            let p = plan(&f, by_hand);
            assert!(by_hand || p.steps.iter().all(|s| !s.route.by_hand()), "{f:?}: {:?}", routes(&p));
            let mut s = State::from(&f);
            for st in &p.steps {
                assert!(applies(st.route, &f, &s), "{f:?}: {:?} doesn't run here", st.route);
                let n = apply(st.route, &s);
                assert!(n.left() < s.left() || n.module != s.module || n.staged != s.staged, "{f:?}: {:?}", st.route);
                s = n;
            }
            assert!(p.steps.len() <= 3, "{f:?}: {:?}", routes(&p));
        }
    }

    #[test]
    fn a_part_once_where_it_can() {
        // the main board written twice only where nothing else gets there: a ZD1L whose main board can't start its
        // updater yet (or a staged app an old rescue would serve) and a boot image behind
        for f in every() {
            let p = plan(&f, false);
            let writes_mcu = |r: Route| {
                matches!(
                    r,
                    Route::SystemFollow | Route::Follow | Route::Dfu | Route::Install | Route::Updater | Route::KeyHold
                )
            };
            let n = p.steps.iter().filter(|s| writes_mcu(s.route)).count();
            if n > 1 {
                assert_eq!(f.model, Model::Zd1l, "{f:?}: {:?}", routes(&p));
                assert!(!f.updater || f.staged, "{f:?}: {:?}", routes(&p));
            }
        }
    }

    #[test]
    fn the_cases_people_have() {
        // the owner's ZD1 over the network, everything behind: the system and the main board in one hold; the boot
        // image needs USB
        let p = plan_auto(&facts(Model::Zd1, Link::Net));
        assert_eq!(routes(&p), [Route::SystemNet]);
        assert!(p.left_boot && !p.left_system && !p.left_mcu);
        // ... over USB: one visit to the bootloader
        assert_eq!(routes(&plan_auto(&facts(Model::Zd1, Link::Usb))), [Route::Install]);
        // ... the boot image current: the system over USB, the main board over DFU (a backup first)
        let f = Facts { boot: false, ..facts(Model::Zd1, Link::Usb) };
        assert_eq!(routes(&plan_auto(&f)), [Route::Dfu, Route::SystemUsb]);
        // a ZD1L on 0.5.1 or later, everything behind, Ethernet: its updater alone
        assert_eq!(routes(&plan_auto(&facts(Model::Zd1l, Link::Usb))), [Route::Updater]);
        assert_eq!(routes(&plan_auto(&facts(Model::Zd1l, Link::Net))), [Route::Updater]);
        // jurajs5's: 0.5.0's system kept its app staged (an old rescue would serve it): the system and the main board
        // from the module first, then the updater
        let f = Facts { staged: true, ..facts(Model::Zd1l, Link::Usb) };
        assert_eq!(routes(&plan_auto(&f)), [Route::SystemFollow, Route::Updater]);
        // a main board too old to start the updater: the same
        let f = Facts { updater: false, ..facts(Model::Zd1l, Link::Usb) };
        assert_eq!(routes(&plan_auto(&f)), [Route::SystemFollow, Route::Updater]);
        // no Ethernet: the system and the main board from the module; the boot image is left, by hand the key-hold
        // does it all
        let f = Facts { ethernet: false, ..facts(Model::Zd1l, Link::Usb) };
        let p = plan_auto(&f);
        assert_eq!(routes(&p), [Route::SystemFollow]);
        assert!(p.left_boot);
        assert_eq!(routes(&plan(&f, true)), [Route::KeyHold]);
        // the boot image current, a ZD1L: the system and the main board from the module, no updater
        let f = Facts { boot: false, ..facts(Model::Zd1l, Link::Usb) };
        assert_eq!(routes(&plan_auto(&f)), [Route::SystemFollow]);
        // up to date: nothing
        let f = Facts { boot: false, system: false, mcu: false, ..facts(Model::Zd1, Link::Usb) };
        assert!(plan_auto(&f).steps.is_empty());
    }
}
