extern crate reqwest;
extern crate serde;
extern crate serde_json;
extern crate console;

use std::result;
use std::fmt;
use std::env;


use reqwest::header::{HeaderMap, HeaderValue};
use serde::Deserialize;
use console::{Style, StyledObject};

const API_KEY: &str = "PLACEHOLDER";
const X_RIOT_TOKEN: &'static str = "X-Riot-Token";
const ACC_COLS: [&str; 5] = ["Level", "Rank", "W/L", "LP", "Hot Streak"];
const GAME_COLS: [&str; 5] = ["Username", "Rank", "LP", "W/L", "Hot Streak"];
const FIRE: &'static str = "🔥";
const COLD: &'static str = "🧊";

type Result<T> = result::Result<T, ProgramError>;

macro_rules! fetch {
    ($url:expr) => {
        {
            let mut headers = HeaderMap::new();
            headers.insert(X_RIOT_TOKEN, HeaderValue::from_static(API_KEY));
            let client = reqwest::Client::new();
            let res = client.get(&$url)
            .headers(headers)
            .send()
            .await.or_else(|_| Err(ProgramError::InvalidUrl));
            res
        }
    };
}

macro_rules! gameHeader {
    ($team:expr, $i:ident) => {
        println!("{:=^59}", $i.apply_to($team));
        println!("{0: ^17} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^10}", GAME_COLS[0], GAME_COLS[1],
        GAME_COLS[2], GAME_COLS[3], GAME_COLS[4]);
        println!("{:-<18}+{:-<8}+{:-<8}+{:-<8}+{:-<12}", "-", "-", "-", "-", "-");
    };
}

macro_rules! emoji {
    ($e:expr, $o:expr) => {
        {
            match utf8_supported() {
                true => $e,
                false => $o,
            }
        }
    };
}


#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        println!("Invalid args. Type 'help' to more info.");
        return Ok(())
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
                    Ok(user) =>  user.display_console(),
                    Err(e) => println!("{}", e),
                }
            } else {
                println!("Must supply username.");
                return Ok(())
            }
        },
        "game" => {
            if let Some(username) = username {
                match look_up_game(&username).await{
                    Ok(game) => game.display_console(),
                    Err(e) => println!("{}", e),
                }
            } else {
                println!("Must supply username.");
                return Ok(())
            }
        },
        "history" => {
            if let Some(username) = username {
                match look_up_history(&username).await{
                    Ok(_game) => (),
                    Err(e) => println!("{}", e),
                }
            } else {
                println!("Must supply username.");
                return Ok(())
            }
        },
        "jelp" => {
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
    let game = create_game(&json.participants, &json.gameMode).await;
    Ok(game)
}



async fn create_game(teammates: &Vec<ParticipantJSON>, mode: &str) -> Game {
    let mut red: Vec<Participant> = Vec::new();
    let mut blue: Vec<Participant> = Vec::new();
    for player in teammates {
        if player.teamId == 100 {
            let rank = get_account_rank(&player.summonerId).await.expect("Error looking up user");
            red.push(Participant::new(false, player.summonerName.clone(), rank));
        } else {
            let rank = get_account_rank(&player.summonerId).await.expect("Error looking up user");
            blue.push(Participant::new(true, player.summonerName.clone(), rank));
        }
    }
    Game {
        red,
        blue,
        mode: mode.to_string(), 
    }
}


async fn look_up_history(username: &str) -> result::Result<(), ProgramError> {
    let account = get_account(username).await?;
    let _history = get_history(&account.accountId).await?;
    println!("Feature not yet finished");
    //println!("{:#?}", history);
    //println!("{}", history.matches.len());
    Ok(())
}

async fn get_history(account_id: &str) -> Result<HistoryJSON>{
    let url = format!("https://na1.api.riotgames.com/lol/match/v4/matchlists/by-account/{}?queue=400&queue=410&queue=420&queue=430&queue=440&endIndex=20", account_id);
    let res = fetch!(url)?;
    match res.status().as_u16() {
        404 => Err(ProgramError::NoHistory),
        200 => {
            let data = res.text().await.or_else(|_| Err(ProgramError::DeserializeError))?;
            Ok(serde_json::from_str(&data[..]).unwrap()) //.or_else(|_| Err(ProgramError::DeserializeError))?)
        },
        _   => Err(ProgramError::InvalidResponse), 
    }
}


async fn look_up_user(username: &str) -> Result<UserAccount> {
    let account = get_account(username).await?;
    let rank = get_account_rank(&account.id).await?;
    Ok(UserAccount::new(account, rank))
}

async fn get_current_game(summoner_id: &str) -> result::Result<GameJSON, ProgramError> {
    let url = String::from("https://na1.api.riotgames.com/lol/spectator/v4/active-games/by-summoner/") + summoner_id;
    let res = fetch!(url)?;
    return match res.status().as_u16() {
        404 => {
            Err(ProgramError::NotInGame)
        },
        200 => {
            let data = res.text().await.or_else(|_| Err(ProgramError::DeserializeError))?;
            Ok(serde_json::from_str(&data[..]).or_else(|_| Err(ProgramError::DeserializeError))?)
        },
        _   => Err(ProgramError::InvalidResponse),
    }
}

async fn get_account_rank(summoner_id: &str) -> Result<Rank> {
    let url = String::from("https://na1.api.riotgames.com/lol/league/v4/entries/by-summoner/") + summoner_id;
    let res = fetch!(url)?;

    match res.status().as_u16() {
        200 => {
            let data = res.text().await.or_else(|_| Err(ProgramError::DeserializeError))?;
            let rank: Vec<Rank> = serde_json::from_str(&data[..]).or_else(|_| Err(ProgramError::DeserializeError))?;
            if let Some(x) = rank.into_iter().filter(|v| v.queueType == "RANKED_SOLO_5x5").collect::<Vec<Rank>>().get(0) {
                Ok(x.clone())
            } else {
                Ok(Rank::unranked())
            }
        },
        404 => Err(ProgramError::InvalidAccount),
        _   => Err(ProgramError::BadResponse),
    }
}


async fn get_account(username: &str) -> result::Result<Account, ProgramError> {
    let url = String::from("https://na1.api.riotgames.com/lol/summoner/v4/summoners/by-name/") + username;
    let res = fetch!(url)?;

    match res.status().as_u16() {
        200 => {
            let data = res.text().await.or_else(|_| Err(ProgramError::DeserializeError))?;
            Ok(serde_json::from_str(&data[..]).or_else(|_| Err(ProgramError::DeserializeError))?)
        },
        404 => Err(ProgramError::InvalidAccount),
        _   => Err(ProgramError::BadResponse),
    }
}

fn utf8_supported() -> bool {
    match std::env::var("LANG") {
        Ok(lang) => lang.to_uppercase().ends_with("UTF-8"),
        _ => false,
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
            ProgramError::NotInGame         => write!(f, "Summoner is not in game."),
            ProgramError::InvalidAccount    => write!(f, "Account does not exist."),
            ProgramError::InvalidUrl        => write!(f, "Invalid url"),
            ProgramError::NotRanked         => write!(f, "Summoner is not ranked in solo's"),
            ProgramError::BadResponse       => write!(f, "Bad response"),
            ProgramError::DeserializeError  => write!(f, "Error deserializing JSON"),
            ProgramError::InvalidResponse   => write!(f, "Invalid response, Status code not 404 or 200"),
            ProgramError::NoHistory         => write!(f, "No history available"),
        }
    }
}




#[derive(Debug)]
struct UserAccount {
    account: Account,
    rank: Rank,
}

impl UserAccount {
    fn new(account: Account, rank: Rank) -> Self {
        UserAccount { account, rank }
    }

    fn display_console(&self) {
        let yellow: Style = Style::new().yellow();
        println!("{:=^47}", yellow.apply_to(&self.account.name));
        println!("{0: ^6} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^10}", ACC_COLS[0], ACC_COLS[1],
        ACC_COLS[2], ACC_COLS[3], ACC_COLS[4]);
        println!("{:-<7}+{:-<8}+{:-<8}+{:-<8}+{:-<12}", "-", "-", "-", "-", "-");
        println!("{0: ^6} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^10}", self.account.summonerLevel,
        self.rank.print_rank(), self.rank.style_wl(), self.rank.leaguePoints, self.rank.display_streak());
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
    summonerId: String
}

#[allow(non_snake_case)]
#[derive(Debug)]
struct Participant {
    team: bool,
    summonerName: String,
    rank: Rank,
}

impl Participant {
    fn new(team: bool, name: String, rank: Rank) -> Self {
        Participant{ team, summonerName: name, rank }
    }
}

#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct HistoryJSON {
    matches: Vec<MatchJSON>
}


#[allow(non_snake_case)]
#[derive(Deserialize, Debug)]
struct MatchJSON {
    queue: u32,
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
}

impl Game {
    fn display_console(&self) {
        let red: Style = Style::new().red();
        let cyan: Style = Style::new().cyan();

        gameHeader!("Red Team", red);
        for person in &self.red {
            Self::display_row(&person);
        }
        println!("\n");
        gameHeader!("Blue Team", cyan);
        for person in &self.blue {
            Self::display_row(&person);
        }
    }

    fn display_row(p: &Participant) {
        println!("{0: <17} | {1: ^6} | {2: ^6} | {3: ^6} | {4: ^10}", p.summonerName,
        p.rank.print_rank(), p.rank.leaguePoints, p.rank.style_wl(), p.rank.display_streak());
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
            return -1.0
        }
        let temp = self.wins as f32 /  (self.wins as f32 + self.losses as f32);
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
            "N/A"           => self.tier.clone(),
            "CHALLENGER"    => "CHAL".to_string(),
            "GRANDMASTER"   => "GRAND".to_string(),
            "MASTER"        => "MAST".to_string(),
            _   => {
                let first_char = self.tier.get(0..1).unwrap();
                first_char.to_string() + "_" +  &self.rank
            }, 
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