extern crate chrono;
extern crate futures;
extern crate regex;
extern crate serde;
extern crate serde_json;

#[macro_use]
extern crate serde_derive;

use futures::Stream;

mod parser;

type DateTime = chrono::DateTime<chrono::Utc>;

#[derive(Clone, Debug, PartialEq)]
enum Language {
    English,
    Japanese,
}

type BossName<'a> = &'a str;

#[derive(Debug, PartialEq)]
pub struct RaidWithBossImage<'a> {
    raid: Raid<'a>,
    image: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
pub struct Raid<'a> {
    pub id: &'a str,
    pub user: &'a str,
    pub user_image: Option<&'a str>,
    pub boss: BossName<'a>,
    pub text: Option<&'a str>,
    pub timestamp: DateTime,
}

trait Components {
    type Source: Stream;
    type Parser: Parser<<Self::Source as Stream>::Item>;
    type Serializer: Serializer;
}

trait Parser<T> {
    fn parse<'a>(&mut self, input: &'a T) -> Option<RaidWithBossImage<'a>>;
}

trait Serializer {
    type Out;

    fn serialize(&mut self, input: Raid) -> Self::Out;
}
