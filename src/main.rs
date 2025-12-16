use axum::{
    Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use moka::future::Cache;
use std::net::SocketAddr;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};

#[derive(Clone)]
struct AppState {
    cache: Cache<String, String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Cache for 30 seconds - reduces MTA API calls significantly
    let cache = Cache::builder()
        .max_capacity(100)
        .time_to_live(Duration::from_secs(30))
        .build();

    let state = AppState { cache };

    // Rate limiting: 10 requests per IP per second
    let governor_conf = GovernorConfigBuilder::default()
        .per_second(10)
        .burst_size(20)
        .finish()
        .ok_or("Failed to build governor config")?;

    let app = Router::new()
        .route("/", get(handle_index))
        .route(
            "/api/calendars/train/:train_name",
            get(handle_train_calendar),
        )
        .layer(
            ServiceBuilder::new()
                .layer(GovernorLayer {
                    config: governor_conf.into(),
                })
                .layer(tower::limit::ConcurrencyLimitLayer::new(50)), // Max 50 concurrent requests
        )
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;

    println!("Server running on http://0.0.0.0:3000");
    println!("Rate limit: 10 req/s per IP, 30s cache, max 50 concurrent requests");
    println!("Example: http://localhost:3000/api/calendars/train/A.ics");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}

async fn handle_train_calendar(
    State(state): State<AppState>,
    Path(train_name): Path<String>,
) -> Response {
    let train_name = train_name.strip_suffix(".ics").unwrap_or(&train_name);

    const VALID_TRAINS: &[&str] = &[
        "A", "C", "E", "B", "D", "F", "M", "G", "J", "Z", "L", "N", "Q", "R", "W", "1", "2", "3",
        "4", "5", "6", "7", "S", "SI",
    ];

    if !VALID_TRAINS.contains(&train_name) {
        return (
            StatusCode::BAD_REQUEST,
            format!(
                "Invalid train line: {}. Train lines are case-sensitive.",
                train_name
            ),
        )
            .into_response();
    }

    // Check cache first
    if let Some(cached_content) = state.cache.get(train_name).await {
        println!("Cache hit for train: {}", train_name);
        return (
            StatusCode::OK,
            [("Content-Type", "text/calendar; charset=utf-8")],
            cached_content,
        )
            .into_response();
    }

    println!("Cache miss - fetching calendar for train: {}", train_name);

    match nyc_train_time::generate_train_ics(train_name).await {
        Ok(ics_content) => {
            // Cache the result
            state
                .cache
                .insert(train_name.to_string(), ics_content.clone())
                .await;

            (
                StatusCode::OK,
                [("Content-Type", "text/calendar; charset=utf-8")],
                ics_content,
            )
                .into_response()
        }
        Err(e) => {
            eprintln!("Error generating calendar: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error generating calendar: {}", e),
            )
                .into_response()
        }
    }
}

async fn handle_index() -> Response {
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>NYC Train Cal</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            max-width: 800px;
            margin: 0 auto;
            padding: 20px;
            line-height: 1.6;
        }
        h1 {
            color: #333;
            margin-top: 20px;
        }
        .line-group {
            margin: 30px 0;
        }
        .train-grid {
            display: grid;
            grid-template-columns: repeat(auto-fill, minmax(80px, 1fr));
            gap: 12px;
            margin-bottom: 20px;
        }
        .train-link {
            display: block;
            padding: 20px;
            text-align: center;
            font-weight: bold;
            font-size: 24px;
            border-radius: 8px;
            transition: transform 0.2s;
            cursor: pointer;
            border: none;
            text-decoration: none;
        }
        .train-link:hover {
            transform: scale(1.05);
        }
        .train-link.selected {
            box-shadow: 0 0 0 3px #333;
        }
        /* NYC Subway line colors */
        .train-1, .train-2, .train-3 { background-color: #ee352e; color: white; }
        .train-4, .train-5, .train-6 { background-color: #00933c; color: white; }
        .train-7 { background-color: #b933ad; color: white; }
        .train-A, .train-C, .train-E { background-color: #0039a6; color: white; }
        .train-B, .train-D, .train-F, .train-M { background-color: #ff6319; color: white; }
        .train-G { background-color: #6cbe45; color: white; }
        .train-J, .train-Z { background-color: #996633; color: white; }
        .train-L { background-color: #a7a9ac; color: white; }
        .train-N, .train-Q, .train-R, .train-W { background-color: #fccc0a; color: black; }
        .train-S, .train-SI { background-color: #808183; color: white; }
        .url-section {
            background-color: #f5f5f5;
            padding: 20px;
            border-radius: 8px;
            margin: 30px 0;
            display: none;
        }
        .url-section.visible {
            display: block;
        }
        .url-container {
            display: flex;
            gap: 10px;
            margin-top: 10px;
        }
        .url-box {
            flex: 1;
            padding: 12px;
            font-family: monospace;
            font-size: 14px;
            background-color: white;
            border: 2px solid #ddd;
            border-radius: 4px;
            word-break: break-all;
        }
        .copy-btn {
            padding: 12px 24px;
            background-color: #0039a6;
            color: white;
            border: none;
            border-radius: 4px;
            cursor: pointer;
            font-weight: bold;
            transition: background-color 0.2s;
        }
        .copy-btn:hover {
            background-color: #002d7a;
        }
        .copy-btn.copied {
            background-color: #00933c;
        }
        .instructions {
            background-color: #f5f5f5;
            padding: 20px;
            border-radius: 8px;
            margin-top: 30px;
        }
        .instructions code {
            background-color: #e0e0e0;
            padding: 2px 6px;
            border-radius: 3px;
            font-family: monospace;
        }
        @media (max-width: 600px) {
            body {
                padding: 15px;
            }
            h1 {
                font-size: 24px;
                margin-top: 10px;
            }
            .train-grid {
                grid-template-columns: repeat(auto-fill, minmax(60px, 1fr));
                gap: 8px;
            }
            .train-link {
                padding: 15px;
                font-size: 20px;
            }
            .url-container {
                flex-direction: column;
            }
            .copy-btn {
                width: 100%;
            }
        }
    </style>
</head>
<body>
    <h1>🚇 NYC Train Cal</h1>
    <p>Subscribe to service alerts for your train line and plan your days better. Click a line to get its calendar subscription URL:</p>
    
    <div class="train-grid">
        <button class="train-link train-A" data-train="A">A</button>
        <button class="train-link train-C" data-train="C">C</button>
        <button class="train-link train-E" data-train="E">E</button>
        <button class="train-link train-B" data-train="B">B</button>
        <button class="train-link train-D" data-train="D">D</button>
        <button class="train-link train-F" data-train="F">F</button>
        <button class="train-link train-M" data-train="M">M</button>
        <button class="train-link train-G" data-train="G">G</button>
        <button class="train-link train-J" data-train="J">J</button>
        <button class="train-link train-Z" data-train="Z">Z</button>
        <button class="train-link train-L" data-train="L">L</button>
        <button class="train-link train-N" data-train="N">N</button>
        <button class="train-link train-Q" data-train="Q">Q</button>
        <button class="train-link train-R" data-train="R">R</button>
        <button class="train-link train-W" data-train="W">W</button>
        <button class="train-link train-1" data-train="1">1</button>
        <button class="train-link train-2" data-train="2">2</button>
        <button class="train-link train-3" data-train="3">3</button>
        <button class="train-link train-4" data-train="4">4</button>
        <button class="train-link train-5" data-train="5">5</button>
        <button class="train-link train-6" data-train="6">6</button>
        <button class="train-link train-7" data-train="7">7</button>
        <button class="train-link train-S" data-train="S">S</button>
        <button class="train-link train-SI" data-train="SI">SI</button>
    </div>
    
    <div class="url-section" id="urlSection">
        <h3>Calendar Subscription URL</h3>
        <p>Copy this URL and add it to your calendar app:</p>
        <div class="url-container">
            <div class="url-box" id="urlBox"></div>
            <button class="copy-btn" id="copyBtn">Copy</button>
        </div>
    </div>
    
    <div class="instructions">
        <h2>How to Subscribe</h2>
        <ol>
            <li>Click on a train line above to get its calendar URL</li>
            <li>Click the "Copy" button to copy the URL</li>
            <li>In your calendar app (Google Calendar, Apple Calendar, Outlook, etc.), look for "Subscribe to calendar" or "Add calendar by URL"</li>
            <li>Paste the URL you copied</li>
            <li>Your calendar will stay updated with MTA service alerts and planned service changes so you can plan ahead!</li>
        </ol>
    </div>

    <script>
        const trainButtons = document.querySelectorAll('.train-link');
        const urlSection = document.getElementById('urlSection');
        const urlBox = document.getElementById('urlBox');
        const copyBtn = document.getElementById('copyBtn');
        
        trainButtons.forEach(button => {
            button.addEventListener('click', () => {
                const train = button.dataset.train;
                const url = window.location.origin + '/api/calendars/train/' + train + '.ics';
                
                // Update selected state
                trainButtons.forEach(btn => btn.classList.remove('selected'));
                button.classList.add('selected');
                
                // Show URL section
                urlSection.classList.add('visible');
                urlBox.textContent = url;
                
                // Reset copy button
                copyBtn.textContent = 'Copy';
                copyBtn.classList.remove('copied');
                
                // Scroll to URL section
                urlSection.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
            });
        });
        
        copyBtn.addEventListener('click', () => {
            const url = urlBox.textContent;
            navigator.clipboard.writeText(url).then(() => {
                copyBtn.textContent = 'Copied!';
                copyBtn.classList.add('copied');
                setTimeout(() => {
                    copyBtn.textContent = 'Copy';
                    copyBtn.classList.remove('copied');
                }, 2000);
            });
        });
    </script>
</body>
</html>"#;

    (
        StatusCode::OK,
        [("Content-Type", "text/html; charset=utf-8")],
        html,
    )
        .into_response()
}
