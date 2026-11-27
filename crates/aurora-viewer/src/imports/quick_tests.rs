use super::*;
use crate::imports::tests::image;
use aurora_net::inventory::InvFolder;

struct Fixture {
    imports: Imports,
    world: crate::world::World,
    inventory: crate::ui::inventory::InventoryPreferences,
    prefs: Preferences,
    prices: Prices,
    ctx: egui::Context,
    jobs: crate::scene::jobs::Jobs,
}

fn fixture(price: i32) -> Fixture {
    let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
    world.inventory.root = Uuid::from_u128(1);
    world.inventory.created_folders(vec![InvFolder {
        id: world.inventory.root,
        name: "Inventory".into(),
        type_default: 8,
        ..Default::default()
    }]);
    world.balance = Some(1000);
    let (jobs, _results) = crate::scene::jobs::Jobs::new();
    Fixture {
        imports: Imports::default(),
        world,
        inventory: Default::default(),
        prefs: Preferences::default(),
        prices: Prices {
            texture: Some(price),
            large_texture: Some(price * 4),
            sound: Some(price),
            animation: Some(price),
        },
        ctx: egui::Context::default(),
        jobs,
    }
}

fn texture(name: &str, side: u32) -> Entry {
    Entry::new(name.into(), Ok(Source::Texture(image(side, side))), Default::default())
}

impl Fixture {
    /// The quick import of `choice` given `entries` by its file chooser.
    fn pick(&mut self, choice: Choice, entries: Vec<Entry>) -> Vec<Uuid> {
        let request = Uuid::new_v4();
        self.imports.pending_files = Some(request);
        self.imports.quick = Some(Quick {
            choice,
            request,
            ids: None,
            ask: None,
            sent: false,
        });
        let ids = entries.iter().map(|entry| entry.id).collect();
        self.imports.tx.send(Worker::Loaded(request, entries)).expect("loaded");
        self.frame();
        ids
    }

    fn frame(&mut self) {
        self.imports.poll(&self.ctx, &self.jobs, &self.world, &self.inventory);
        self.imports
            .advance_quick(&self.ctx, &self.world, &self.inventory, &self.prefs, &self.prices);
    }

    fn answer(&mut self, answer: Answer) {
        self.imports.answer_quick(
            answer,
            &self.ctx,
            &self.jobs,
            &self.world,
            &self.inventory,
            &self.prefs,
            &self.prices,
        );
    }

    fn ask(&self) -> Option<Ask> {
        self.imports.quick.as_ref().and_then(|quick| quick.ask)
    }

    /// The entries the upload took, in its queue.
    fn sent(&self) -> Vec<Uuid> {
        self.imports
            .current
            .iter()
            .map(|(_, step)| step)
            .chain(&self.imports.queue)
            .filter_map(|step| step.entry)
            .collect()
    }
}

#[test]
fn paid_files_ask_their_price_once_and_go_up_alone() {
    let mut f = fixture(10);
    // a texture already in the import list stays there, selected
    let listed = texture("listed.png", 16);
    let listed_id = listed.id;
    f.imports.entries.push(listed);
    let picked = f.pick(Choice::Texture, vec![texture("a.png", 16), texture("b.png", 16)]);
    assert_eq!(f.ask(), Some(Ask::Price { count: 2, cost: 20 }));
    assert!(!f.imports.busy(), "nothing sent before the answer");
    assert!(!f.imports.window_open(Choice::Texture), "no list");
    f.answer(Answer::Upload(None));
    assert!(f.imports.busy());
    let mut sent = f.sent();
    sent.sort();
    let mut expected = picked.clone();
    expected.sort();
    assert_eq!(sent, expected, "the files picked, not the list's");
    let listed = f.imports.entries.iter().find(|entry| entry.id == listed_id).expect("kept");
    assert!(listed.selected && matches!(listed.status, Status::Ready));
}

#[test]
fn free_files_go_up_at_once() {
    let mut f = fixture(0);
    f.pick(Choice::Texture, vec![texture("a.png", 16)]);
    assert_eq!(f.ask(), None);
    assert!(f.imports.busy(), "sent without a question");
}

#[test]
fn images_that_could_be_2048_ask_the_size_with_both_prices() {
    for (large, maximum) in [(false, 1024), (true, 2048)] {
        let mut f = fixture(10);
        f.pick(Choice::Texture, vec![texture("big.png", 2048), texture("small.png", 16)]);
        assert_eq!(
            f.ask(),
            Some(Ask::Size {
                count: 2,
                small: 20,
                large: 50
            })
        );
        f.answer(Answer::Upload(Some(large)));
        assert!(f.imports.busy());
        let big = f.imports.entries.iter().find(|entry| entry.name == "big").expect("big");
        assert_eq!(f.imports.options(big).max_width, maximum);
    }
    // free too: the size is still a question
    let mut f = fixture(0);
    f.pick(Choice::Texture, vec![texture("big.png", 2048)]);
    assert_eq!(
        f.ask(),
        Some(Ask::Size {
            count: 1,
            small: 0,
            large: 0
        })
    );
}

#[test]
fn free_images_can_go_up_at_2048_without_the_question() {
    for (price, asked) in [(0, false), (10, true)] {
        let mut f = fixture(price);
        f.prefs.quick_free_full_size = true;
        f.pick(Choice::Texture, vec![texture("big.png", 2048)]);
        assert_eq!(f.ask().is_some(), asked, "price {price}");
        assert_eq!(f.imports.busy(), !asked);
        if !asked {
            let big = f.imports.entries.iter().find(|entry| entry.name == "big").expect("big");
            assert_eq!(f.imports.options(big).max_width, 2048);
        }
    }
}

#[test]
fn cancelling_takes_the_files_picked_out_of_the_queue() {
    let mut f = fixture(10);
    let listed = texture("listed.png", 16);
    f.imports.entries.push(listed);
    let picked = f.pick(Choice::Texture, vec![texture("a.png", 16)]);
    f.answer(Answer::Cancel);
    assert!(f.imports.quick.is_none());
    assert!(!f.imports.busy());
    assert_eq!(f.imports.entries.len(), 1, "the list's file stays");
    assert!(f.imports.entries.iter().all(|entry| !picked.contains(&entry.id)));
}

#[test]
fn a_chooser_closed_with_nothing_ends_the_quick_import() {
    let mut f = fixture(10);
    let request = Uuid::new_v4();
    f.imports.quick = Some(Quick {
        choice: Choice::Texture,
        request,
        ids: None,
        ask: None,
        sent: false,
    });
    f.imports.pending_files = Some(request);
    f.frame();
    assert!(f.imports.quick.is_some(), "the chooser is still open");
    f.imports.pending_files = None;
    f.frame();
    assert!(f.imports.quick.is_none());
}

#[test]
fn what_cannot_go_up_as_it_is_opens_its_window() {
    // a file that failed to read
    let mut f = fixture(10);
    let broken = Entry::new("broken.png".into(), Err("unreadable".into()), Default::default());
    f.pick(Choice::Texture, vec![texture("a.png", 16), broken]);
    assert!(f.imports.quick.is_none());
    assert!(f.imports.window_open(Choice::Texture));
    assert!(!f.imports.busy());
    // a sound longer than the grid takes, to cut in the window
    let mut f = fixture(10);
    let long = Source::Sound(Arc::new(aurora_assets::import_sound::Sound {
        source_channels: 1,
        source_bits: 16,
        mono: vec![0.0; 44100 * 40],
    }));
    f.pick(Choice::Sound, vec![Entry::new("long.wav".into(), Ok(long), Default::default())]);
    assert!(f.imports.quick.is_none());
    assert!(f.imports.window_open(Choice::Sound));
    // a sound that fits asks its price
    let mut f = fixture(10);
    let short = Source::Sound(Arc::new(aurora_assets::import_sound::Sound {
        source_channels: 1,
        source_bits: 16,
        mono: vec![0.0; 44100],
    }));
    f.pick(Choice::Sound, vec![Entry::new("bell.wav".into(), Ok(short), Default::default())]);
    assert_eq!(f.ask(), Some(Ask::Price { count: 1, cost: 10 }));
}

#[test]
fn a_balance_too_low_is_refused_before_anything_is_sent() {
    let mut f = fixture(10);
    f.world.balance = Some(5);
    f.pick(Choice::Texture, vec![texture("a.png", 16)]);
    assert_eq!(f.ask(), Some(Ask::Price { count: 1, cost: 10 }));
    // the question's OK is greyed out; an answer all the same is refused
    f.answer(Answer::Upload(None));
    assert!(!f.imports.busy());
    assert!(f.imports.quick.is_none());
    assert!(f.imports.window_open(Choice::Texture), "the window says why");
}

#[test]
fn the_quick_import_ends_once_its_files_are_uploaded() {
    let mut f = fixture(0);
    let picked = f.pick(Choice::Texture, vec![texture("a.png", 16)]);
    assert!(f.imports.quick.as_ref().is_some_and(|quick| quick.sent));
    // uploaded: the queue retires it
    f.imports.current = None;
    f.imports.queue.clear();
    for entry in f.imports.entries.iter_mut().filter(|entry| picked.contains(&entry.id)) {
        entry.status = Status::Done(Uuid::new_v4());
    }
    f.frame();
    assert!(f.imports.quick.is_none());
    assert!(!f.imports.window_open(Choice::Texture));
}
