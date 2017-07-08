use circular_buffer::CircularBuffer;
use error::*;
use futures::{Async, Future, Poll, Stream};
use futures::stream::{Map, OrElse, Select};
use futures::unsync::mpsc;
use futures::unsync::oneshot;
use raid::{BossImageUrl, BossLevel, BossName, DateTime, Language, RaidInfo, RaidTweet};
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::sync::Arc;

const DEFAULT_BOSS_LEVEL: BossLevel = 0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RaidBoss {
    pub name: BossName,
    pub level: BossLevel,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<BossImageUrl>,
    pub language: Language,
}

#[derive(Clone, Debug, PartialEq)]
struct RaidBossEntry {
    boss: RaidBoss,
    last_seen: DateTime,
    recent_tweets: CircularBuffer<Arc<RaidTweet>>, // TODO: broadcast
}

#[derive(Debug)]
enum Event {
    NewRaidInfo(RaidInfo),
    GetBosses(oneshot::Sender<Vec<RaidBoss>>),
    GetRecentTweets {
        boss_name: BossName,
        sender: oneshot::Sender<Vec<Arc<RaidTweet>>>,
    },
    ReadError,
}

pub struct AsyncResult<T>(oneshot::Receiver<T>);
impl<T> Future for AsyncResult<T> {
    type Item = T;
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll().map_err(|_| ErrorKind::Closed.into())
    }
}

#[derive(Clone, Debug)]
pub struct Petronel(mpsc::UnboundedSender<Event>);
impl Petronel {
    fn request<T, F>(&self, f: F) -> AsyncResult<T>
    where
        F: FnOnce(oneshot::Sender<T>) -> Event,
    {
        let (tx, rx) = oneshot::channel();
        let _ = mpsc::UnboundedSender::send(&self.0, f(tx));
        AsyncResult(rx)
    }

    pub fn bosses(&self) -> AsyncResult<Vec<RaidBoss>> {
        self.request(Event::GetBosses)
    }

    pub fn recent_tweets<B>(&self, boss_name: B) -> AsyncResult<Vec<Arc<RaidTweet>>>
    where
        B: AsRef<str>,
    {
        self.request(|tx| {
            Event::GetRecentTweets {
                boss_name: BossName::new(boss_name),
                sender: tx,
            }
        })
    }
}

pub struct PetronelFuture<S> {
    events: Select<
        Map<S, fn(RaidInfo) -> Event>,
        OrElse<mpsc::UnboundedReceiver<Event>, fn(()) -> Result<Event>, Result<Event>>,
    >,
    bosses: HashMap<BossName, RaidBossEntry>,
    tweet_history_size: usize,
}

impl Petronel {
    fn events_read_error(_: ()) -> Result<Event> {
        Ok(Event::ReadError)
    }

    // TODO: Builder
    pub fn from_stream<S>(stream: S, tweet_history_size: usize) -> (Self, PetronelFuture<S>)
    where
        S: Stream<Item = RaidInfo, Error = Error>,
    {
        let (tx, rx) = mpsc::unbounded();

        let stream_events = stream.map(Event::NewRaidInfo as fn(RaidInfo) -> Event);
        let rx = rx.or_else(Self::events_read_error as fn(()) -> Result<Event>);

        let future = PetronelFuture {
            events: stream_events.select(rx),
            bosses: HashMap::new(),
            tweet_history_size,
        };

        (Petronel(tx), future)
    }
}

impl<S> PetronelFuture<S> {
    fn handle_event(&mut self, event: Event) {
        use self::Event::*;

        match event {
            NewRaidInfo(r) => {
                self.handle_raid_info(r);
            }
            GetBosses(tx) => {
                let _ = tx.send(
                    self.bosses
                        .values()
                        .cloned()
                        .map(|e| e.boss)
                        .collect::<Vec<_>>(),
                );
            }
            GetRecentTweets { boss_name, sender } => {
                let backlog = self.bosses.get(&boss_name).map_or(vec![], |e| {
                    // Returns recent tweets, unsorted. The client is
                    // expected to do the sorting on their end.
                    e.recent_tweets.as_unordered_slice().to_vec()
                });

                let _ = sender.send(backlog);
            }
            ReadError => {} // This should never happen
        }
    }

    fn handle_raid_info(&mut self, info: RaidInfo) {
        match self.bosses.entry(info.tweet.boss_name.clone()) {
            Entry::Occupied(mut entry) => {
                let value = entry.get_mut();

                value.last_seen = info.tweet.created_at;
                value.recent_tweets.push(Arc::new(info.tweet));

                if value.boss.image.is_none() && info.image.is_some() {
                    // TODO: Image hash
                    value.boss.image = info.image;
                }
            }
            Entry::Vacant(entry) => {
                let name = entry.key().clone();

                let boss = RaidBoss {
                    level: name.parse_level().unwrap_or(DEFAULT_BOSS_LEVEL),
                    name: name,
                    image: info.image,
                    language: info.tweet.language,
                };

                entry.insert(RaidBossEntry {
                    boss,
                    last_seen: info.tweet.created_at.clone(),
                    recent_tweets: {
                        let mut recent_tweets =
                            CircularBuffer::with_capacity(self.tweet_history_size);
                        recent_tweets.push(Arc::new(info.tweet));
                        recent_tweets
                    },
                });

            }
        }
    }
}

impl<S> Future for PetronelFuture<S>
where
    S: Stream<Item = RaidInfo, Error = Error>,
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if let Some(event) = try_ready!(self.events.poll()) {
                self.handle_event(event)
            } else {
                return Ok(Async::Ready(()));
            }
        }
    }
}
