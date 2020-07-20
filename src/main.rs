extern crate console;
extern crate reqwest;
extern crate serde;
extern crate serde_json;

mod champ;

use champ::champion_map;
use console::{Style, StyledObject};
use futures::future::{join, join_all};
use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::fmt;
use std::result;
use std::{thread, time};
use std::convert::From;

const API_KEY: &str = "";
const X_RIOT_TOKEN: &'static str = "X-Riot-Token";
const ACC_COLS: [&str; 6] = ["Level", "Rank", "W/L", "LP", "Hot Streak", "Top Role"];
const GAME_COLS: [&str; 6] = ["Username", "Rank", "LP", "W/L", "Champion", "Hot Streak"];
const MATCH_HISTORY_COLS: [&str; 4] = ["Role", "Mode", "Champion", "Outcome"];
const FIRE: &'static str = "🔥";
const COLD: &'static str = "🧊";
const DEFAULT_CHAMP: &'static str = "Unknown Champ";
const SLEEP_DUR: u64 = 300;

type Result<T> = result::Result<T, ProgramError>;

macro_rules! fetch {
    ($url:expr) => {{
        let mut headers = HeaderMap::new();
        headers.insert(X_RIOT_TOKEN, HeaderValue::from_static(API_KEY));
        let client = reqwest::Client::new();
        let res = client
            .get(&$url)
            .headers(headers)
            .send()
            .await
            .or_else(|_| Err(ProgramError::InvalidUrl));
        res
    }};
}

macro_rules! gameHeader {
    ($team:expr, $i:ident) => {
        println!("{:=^81}", $i.apply_to($team));
        println!(
            "{0: ^17} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^20} | {5: ^10}",
            GAME_COLS[0], GAME_COLS[1], GAME_COLS[2], GAME_COLS[3], GAME_COLS[4], GAME_COLS[5]
        );
        println!(
            "{:-<18}+{:-<8}+{:-<8}+{:-<8}+{:-<22}+{:-<12}",
            "-", "-", "-", "-", "-", "-"
        );
    };
}

macro_rules! emoji {
    ($e:expr, $o:expr) => {{
        match utf8_supported() {
            true => $e,
            false => $o,
        }
    }};
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Invalid args. Type 'help' to more info.");
        return Ok(());
    }

    let username = match args.len() {
        1 | 2 => None,
        3 => Some(args.get(2).unwrap().clone()),
        _ => Some(args.iter().skip(2).fold(String::new(), |acc, x| acc + x)),
    };

    match &args[1][..] {
        "lookup" => {
            if let Some(username) = username {
                match look_up_user(&username).await {
                    Ok(user) => user.display_console(),
                    Err(e) => println!("{}", e),
                }
            } else {
                println!("Must supply username.");
                return Ok(());
            }
        }
        "game" => {
            if let Some(username) = username {
                match look_up_game(&username).await {
                    Ok(game) => game.display_console(),
                    Err(e) => println!("{}", e),
                }
            } else {
                println!("Must supply username.");
                return Ok(());
            }
        }
        "history" => {
            if let Some(username) = username {
                match look_up_match_history(&username).await {
                    Ok(history) => history.display_console(),
                    Err(e) => println!("{}", e),
                }
                return Ok(());
            } else {
                println!("Must supply username.");
                return Ok(());
            }
        }
        "help" => {
            println!("Available commands:");
            println!("  lookup <username>      => returns account statistics");
            println!("  game <username>        => returns data about current game");
            println!("  history <username>     => returns match history");
        }
        _ => println!("Invalid argument. Type help to see list of args."),
    }

    Ok(())
}

async fn look_up_game(username: &str) -> Result<Game> {
    let account = get_account(username).await?;
    let json = get_current_game(&account.id).await?;
    let game = create_game(&json.participants, &json.gameMode, &json.gameType).await;
    Ok(game)
}

async fn create_game(teammates: &Vec<ParticipantJSON>, mode: &str, game_type: &str) -> Game {
    let futures = teammates
        .iter()
        .map(|p| get_account_rank(&p.summonerId))
        .collect::<Vec<_>>();
    let result = join_all(futures).await;

    let mut blue: Vec<Participant> = Vec::new();
    let mut red: Vec<Participant> = Vec::new();
    for (player, data) in teammates.iter().zip(result.into_iter()) {
        let rank = data.expect("Error looking up user");
        if player.teamId == 100 {
            red.push(Participant::new(false, player.summonerName.clone(), rank, player.championId));
        } else {
            blue.push(Participant::new(false, player.summonerName.clone(), rank, player.championId));
        }
    }
    Game {
        red,
        blue,
        mode: mode.to_string(),
        game_type: game_type.to_string(),
    }
}

// Returns the most played role
async fn get_most_played_role(account_id: &str) -> result::Result<String, ProgramError> {
    let history = get_history(account_id).await?;
    let mut map: HashMap<String, i8> = HashMap::new();

    let mut adc = 0;
    let mut sup = 0;
    for game in history.matches {
        if let Some(r) = map.get_mut(&game.lane) {
            if game.lane == "BOTTOM" {
                if game.role == "DUO_CARRY" {
                    adc += 1;
                } else {
                    sup += 1;
                }
            }
            *r += 1;
        } else {
            map.insert(game.lane, 1);
        }
    }

    let mut max = ("", -1);
    for (key, value) in map.iter() {
        if *value > max.1 {
            max = (key, *value);
        }
    }

    match max.0 {
        "BOTTOM" if adc > sup => Ok(String::from("ADC")),
        "BOTTOM" => Ok(String::from("SUPPORT")),
        _ => Ok(String::from(max.0)),
    }
}

async fn get_history(account_id: &str) -> Result<HistoryJSON> {
    let url = format!("https://na1.api.riotgames.com/lol/match/v4/matchlists/by-account/{}?queue=400&queue=410&queue=420&queue=430&queue=440&endIndex=20", account_id);
    let res = fetch!(url)?;
    match res.status().as_u16() {
        404 => Err(ProgramError::NoHistory),
        200 => {
            let data = res
                .text()
                .await
                .or_else(|_| Err(ProgramError::DeserializeError))?;
            Ok(serde_json::from_str(&data[..]).unwrap()) //.or_else(|_| Err(ProgramError::DeserializeError))?)
        }
        _ => {
            println!("{}", res.status().as_u16());
            return Err(ProgramError::InvalidResponse);
        }
    }
}

async fn look_up_user(username: &str) -> Result<UserAccount> {
    let account = get_account(username).await?;
    let rank = get_account_rank(&account.id);
    let role = get_most_played_role(&account.accountId);
    let result = join(rank, role).await;
    if let (Ok(rank), Ok(role)) = result {
        return Ok(UserAccount::new(account, rank, role));
    }
    Err(ProgramError::InvalidResponse)
}

async fn get_current_game(summoner_id: &str) -> result::Result<GameJSON, ProgramError> {
    let url =
        String::from("https://na1.api.riotgames.com/lol/spectator/v4/active-games/by-summoner/")
            + summoner_id;
    let res = fetch!(url)?;
    return match res.status().as_u16() {
        404 => Err(ProgramError::NotInGame),
        200 => {
            let data = res
                .text()
                .await
                .or_else(|_| Err(ProgramError::DeserializeError))?;
            Ok(serde_json::from_str(&data[..]).or_else(|_| Err(ProgramError::DeserializeError))?)
        }
        _ => Err(ProgramError::InvalidResponse),
    };
}

fn determine_role(role: &str, lane: &str) -> String {
    match lane {
        "BOTTOM" => match role {
            "DUO_SUPPORT" => "SUPPORT".to_string(),
            _ => "ADC".to_string(),
        },
        _ => lane.to_string(),
    }
}

// retrieves the match data for a given id
async fn get_match_data(match_id: u64) -> Result<MatchDataJSON> {
    let url =
        String::from("https://na1.api.riotgames.com/lol/match/v4/matches/") + &match_id.to_string();
    let res = fetch!(url)?;

    match res.status().as_u16() {
        200 => {
            let data = res
                .text()
                .await
                .or_else(|_| Err(ProgramError::DeserializeError))?;
            let match_data: MatchDataJSON =
                serde_json::from_str(&data[..]).or_else(|_| Err(ProgramError::DeserializeError))?;
            Ok(match_data)
        }
        404 => Err(ProgramError::InvalidAccount),
        _ => Err(ProgramError::BadResponse),
    }
}

async fn look_up_match_history(username: &str) -> Result<UserGames> {
    let account = get_account(username).await?;
    let history = get_history(&account.accountId).await?;
    // Need to pause the main thread so the API key usage does not exceed the limit
    let sleep_time = time::Duration::from_millis(SLEEP_DUR);
    thread::sleep(sleep_time);
    let games = history
        .matches
        .iter()
        .map(|m| get_match_data(m.gameId))
        .collect::<Vec<_>>();
    let result = join_all(games).await;

    let mut recent_games = Vec::new();
    for (g, m) in history.matches.iter().zip(result.into_iter()) {
        if let Ok(m) = m {
            let is_win = match &m.teams[0].win[..] {
                "Win" => true,
                _ => false,
            };

            //see if it was a win for the user.
            let par_pos = m
                .participantIdentities
                .iter()
                .position(|p| p.player.accountId == account.accountId)
                .unwrap();
            let id = m.participantIdentities[par_pos].participantId;
            let mut res = false; // Result of the user winning or losing
            for p in m.participants {
                if p.participantId == id {
                    res = match p.teamId {
                        100 => is_win,
                        _ => !is_win,
                    };
                    break;
                }
            }
            recent_games.push(UserMatch::new(
                determine_role(&g.role, &g.lane),
                g.queue,
                g.champion,
                Some(res),
            ))
        } else {
            recent_games.push(UserMatch::new(
                determine_role(&g.role, &g.lane),
                g.queue,
                g.champion,
                None,
            ))
        }
    }
    // let recent_games: Vec<UserMatch> = history.matches.into_iter()
    //     .map(|game| UserMatch::new(determine_role(&game.role, &game.lane), game.queue, game.champion, res))
    //     .collect();
    Ok(UserGames {
        games: recent_games,
        username: String::from(username),
    })
}

async fn get_account_rank(summoner_id: &str) -> Result<Rank> {
    let url = String::from("https://na1.api.riotgames.com/lol/league/v4/entries/by-summoner/")
        + summoner_id;
    let res = fetch!(url)?;

    match res.status().as_u16() {
        200 => {
            let data = res
                .text()
                .await
                .or_else(|_| Err(ProgramError::DeserializeError))?;
            let rank: Vec<Rank> =
                serde_json::from_str(&data[..]).or_else(|_| Err(ProgramError::DeserializeError))?;
            if let Some(x) = rank
                .into_iter()
                .filter(|v| v.queueType == "RANKED_SOLO_5x5")
                .collect::<Vec<Rank>>()
                .get(0)
            {
                Ok(x.clone())
            } else {
                Ok(Rank::unranked())
            }
        }
        404 => Err(ProgramError::InvalidAccount),
        _ => Err(ProgramError::BadResponse),
    }
}

async fn get_account(username: &str) -> result::Result<Account, ProgramError> {
    let url =
        String::from("https://na1.api.riotgames.com/lol/summoner/v4/summoners/by-name/") + username;
    let res = fetch!(url)?;

    match res.status().as_u16() {
        200 => {
            let data = res
                .text()
                .await
                .or_else(|_| Err(ProgramError::DeserializeError))?;
            Ok(serde_json::from_str(&data[..]).or_else(|_| Err(ProgramError::DeserializeError))?)
        }
        404 => Err(ProgramError::InvalidAccount),
        _ => Err(ProgramError::BadResponse),
    }
}

fn utf8_supported() -> bool {
    match std::env::var("LANG") {
        Ok(lang) => lang.to_uppercase().ends_with("UTF-8"),
        _ => false,
    }
}

fn format_game_id(id: u16) -> String {
    match id {
        400 => "Normal Draft".to_string(),
        410 => "Ranked Dynamic".to_string(),
        420 => "Ranked Solo".to_string(),
        430 => "Blink Pick".to_string(),
        440 => "Ranked Flex".to_string(),
        _ => "Unknown".to_string(),
    }
}

#[derive(Debug)]
pub enum ProgramError {
    DeserializeError,
    InvalidUrl,
    NotInGame,
    NotRanked,
    InvalidResponse,
    InvalidAccount,
    BadResponse,
    NoHistory,
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ProgramError::NotInGame => write!(f, "Summoner is not in game."),
            ProgramError::InvalidAccount => write!(f, "Account does not exist."),
            ProgramError::InvalidUrl => write!(f, "Invalid url"),
            ProgramError::NotRanked => write!(f, "Summoner is not ranked in solo's"),
            ProgramError::BadResponse => write!(f, "Bad response"),
            ProgramError::DeserializeError => write!(f, "Error deserializing JSON"),
            ProgramError::InvalidResponse => {
                write!(f, "Invalid response, Status code not 404 or 200")
            }
            ProgramError::NoHistory => write!(f, "No history available"),
        }
    }
}

// Wrapper struct to display the user games
struct UserGames {
    username: String,
    games: Vec<UserMatch>,
}

impl UserGames {
    fn display_console(&self) -> () {
        let yellow: Style = Style::new().yellow();
        let label = format!(" {} Match History ", &self.username);
        let map = champion_map();
        let (wins, losses) = self.games.iter()
            .fold((0,0), |(x,y), game| {
                if let Some(g) = game.outcome {
                    return if g {
                        (x + 1, y)
                    } else {
                        (x, y + 1)
                    };
                }
                (x, y)
            });
        println!("{:=^68}", yellow.apply_to(&label));
        println!("Last 20 games stats:");
        println!("Total wins: {}", wins);
        println!("Total losses: {}", losses);
        println!("W/L Ratio: {:.2}%", (wins as f32 / 20.0) * 100.0);
        println!(
            "{0: ^10} | {1: ^15} | {2: ^20} | {3: ^15}",
            MATCH_HISTORY_COLS[0],
            MATCH_HISTORY_COLS[1],
            MATCH_HISTORY_COLS[2],
            MATCH_HISTORY_COLS[3]
        );
        println!("{:-<11}+{:-<17}+{:-<22}+{:-<16}", "-", "-", "-", "-");
        for game in &self.games {
            Self::display_row(game, &map);
        }
    }

    fn display_row(game: &UserMatch, map: &HashMap<u16, String>) {
        let temp = String::from(DEFAULT_CHAMP);
        let champ = map.get(&game.champ).unwrap_or_else(|| &temp);
        println!(
            "{0: ^10} | {1: ^15} | {2: ^20} | {3: ^15}",
            game.role,
            format_game_id(game.game_mode),
            champ,
            game.get_outcome()
        );
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
// Represents a match fetch given the match id
struct MatchDataJSON {
    teams: Vec<TeamJSON>,
    participants: Vec<MatchParticipantJSON>,
    participantIdentities: Vec<ParticipantIdentityJSON>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct TeamJSON {
    teamId: u16,
    win: String,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct MatchParticipantJSON {
    participantId: u16,
    teamId: u16,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct ParticipantIdentityJSON {
    participantId: u16,
    player: PlayerJSON,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct PlayerJSON {
    accountId: String,
    summonerName: String,
    summonerId: String,
}

#[derive(Debug)]
// A single game that the user played in
struct UserMatch {
    role: String,
    game_mode: u16,
    champ: u16,
    outcome: Option<bool>, // true if a win
}

impl UserMatch {
    //TODO: Finish implementation
    fn new(role: String, mode: u16, champ: u16, outcome: Option<bool>) -> Self {
        UserMatch {
            role: role,
            game_mode: mode,
            champ: champ,
            outcome: outcome,
        }
    }

    fn get_outcome(&self) -> &str {
        match self.outcome {
            Some(v) => match v {
                true => "Win",
                _ => "Loss",
            },
            None => "Unavaliable",
        }
    }
}

#[derive(Debug)]
struct UserAccount {
    account: Account,
    rank: Rank,
    top_role: String,
}

impl UserAccount {
    fn new(account: Account, rank: Rank, top_role: String) -> Self {
        UserAccount {
            account,
            rank,
            top_role,
        }
    }

    fn display_console(&self) {
        let yellow: Style = Style::new().yellow();
        println!("{:=^58}", yellow.apply_to(&self.account.name));
        println!(
            "{0: ^6} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^10} | {5: ^10}",
            ACC_COLS[0], ACC_COLS[1], ACC_COLS[2], ACC_COLS[3], ACC_COLS[4], ACC_COLS[5]
        );
        println!(
            "{:-<7}+{:-<8}+{:-<8}+{:-<8}+{:-<12}+{:-<10}",
            "-", "-", "-", "-", "-", "-"
        );
        if utf8_supported() {
            println!(
                "{0: ^6} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^9} | {5: ^10}",
                self.account.summonerLevel,
                self.rank.print_rank(),
                self.rank.style_wl(),
                self.rank.leaguePoints,
                self.rank.display_streak(),
                self.top_role
            );
        } else {
            println!(
                "{0: ^6} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^10} | {5: ^10}",
                self.account.summonerLevel,
                self.rank.print_rank(),
                self.rank.style_wl(),
                self.rank.leaguePoints,
                self.rank.display_streak(),
                self.top_role
            );
        }
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct Account {
    id: String,
    accountId: String,
    puuid: String,
    name: String,
    profileIconId: i32,
    revisionDate: i64,
    summonerLevel: i32,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct ParticipantJSON {
    teamId: i32,
    summonerName: String,
    summonerId: String,
    championId: u16,
}

#[allow(non_snake_case)]
#[derive(Debug)]
struct Participant {
    team: bool,
    summonerName: String,
    championId: u16,
    rank: Rank,
}

impl Participant {
    fn new(team: bool, name: String, rank: Rank, champ_id: u16) -> Self {
        Participant {
            team,
            summonerName: name,
            championId: champ_id,
            rank,
        }
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct HistoryJSON {
    matches: Vec<MatchJSON>,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct MatchJSON {
    queue: u16,
    season: u8,
    role: String,
    lane: String,
    champion: u16,
    gameId: u64,
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct GameJSON {
    gameMode: String,
    gameType: String,
    participants: Vec<ParticipantJSON>,
}

#[derive(Debug)]
struct Game {
    red: Vec<Participant>,
    blue: Vec<Participant>,
    mode: String,
    game_type: String,
}

impl Game {
    fn display_console(&self) {
        let champ_map = champ::champion_map();
        let rank_map = champ::rank_map();
        let red: Style = Style::new().red();
        let cyan: Style = Style::new().cyan();
        let avg_red = self.red.iter().fold(0, |acc, v| {
            acc + rank_map.get(&v.rank.print_rank()).unwrap()
        }) / 5;
        let avg_blue = self.blue.iter().fold(0, |acc, v| {
            acc + rank_map.get(&v.rank.print_rank()).unwrap()
        }) / 5;
        let mut blue_rank = "N/A";
        let mut red_rank = "N/A";
        for (key, val) in rank_map.iter() {
            if *val == avg_red {
                red_rank = key;
            }
            if *val == avg_blue {
                blue_rank = key;
            }
        }

        println!("Game Mode: {}", self.mode); // Ranked: CLASSIC MATCHED_GAME    ARAM MATCHED_GAME
        println!("Game Type: {}", self.game_type);
        println!("Avg Team Rank: {}", red_rank);
        gameHeader!("Red Team", red);
        for person in &self.red {
            Self::display_row(&person, &champ_map);
        }
        println!("\n");
        println!("Avg Team Rank: {}", blue_rank);
        gameHeader!("Blue Team", cyan);
        for person in &self.blue {
            Self::display_row(&person, &champ_map);
        }
    }

    fn display_row(p: &Participant, map: &HashMap<u16, String>) {
        println!(
            "{0: <17} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^20} | {5}",
            p.summonerName,
            p.rank.print_rank(),
            p.rank.leaguePoints,
            p.rank.style_wl(),
            map.get(&p.championId).unwrap(),
            p.rank.display_streak(),
        );
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug, Clone)]
struct Rank {
    tier: String,
    rank: String,
    queueType: String,
    wins: i32,
    losses: i32,
    hotStreak: bool,
    leaguePoints: i16,
}
impl Rank {
    fn unranked() -> Self {
        Rank {
            tier: String::from("N/A"),
            rank: String::from("N/A"),
            queueType: String::from("N/A"),
            wins: -1,
            losses: -1,
            hotStreak: false,
            leaguePoints: -1,
        }
    }
}

impl Rank {
    fn get_wl_ratio(&self) -> f32 {
        if self.wins == -1 {
            return -1.0;
        }
        let temp = self.wins as f32 / (self.wins as f32 + self.losses as f32);
        temp * 100.0
    }

    fn style_wl(&self) -> StyledObject<String> {
        let temp = self.get_wl_ratio();
        let red = Style::new().red();
        let green = Style::new().green();
        let default = Style::new();
        match temp {
            temp if temp == -1.0 => default.apply_to("N/A".to_string()),
            temp if temp > 55.0 => green.apply_to(format!("{0:.2}%", temp)),
            temp if temp < 48.0 => red.apply_to(format!("{0:.2}%", temp)),
            _ => default.apply_to(format!("{0:.2}%", temp)),
        }
    }

    fn print_rank(&self) -> String {
        match &self.tier[..] {
            "N/A" => self.tier.clone(),
            "CHALLENGER" => "CHAL".to_string(),
            "GRANDMASTER" => "GRAND".to_string(),
            "MASTER" => "MAST".to_string(),
            _ => {
                let first_char = self.tier.get(0..1).unwrap();
                first_char.to_string() + "_" + &self.rank
            }
        }
    }

    fn display_streak(&self) -> &str {
        if self.hotStreak {
            emoji!(FIRE, "Y")
        } else {
            emoji!(COLD, "N")
        }
    }
}