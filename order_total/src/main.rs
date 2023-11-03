#[macro_use]
extern crate lazy_static;

use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, Server, StatusCode};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, num::ParseFloatError};
use std::{error::Error, net::SocketAddr};

lazy_static! {
    static ref SALES_TAX_RATE_SERVICE: String = {
        if let Ok(url) = std::env::var("SALES_TAX_RATE_SERVICE") {
            url
        } else {
            "http://localhost:8001/find_rate".into()
        }
    };
}

#[derive(Serialize, Deserialize, Debug)]
struct Order {
    order_id: i32,
    product_id: i32,
    quantity: i32,
    subtotal: f32,
    shipping_address: String,
    shipping_zip: String,
    total: f32,
}

/*
impl Order {
    fn new(
        order_id: i32,
        product_id: i32,
        quantity: i32,
        subtotal: f32,
        shipping_address: String,
        shipping_zip: String,
        total: f32,
    ) -> Self {
        Self {
            order_id,
            product_id,
            quantity,
            subtotal,
            shipping_address,
            shipping_zip,
            total,
        }
    }
}
*/

/// This is our service handler. It receives a Request, routes on its
/// path, and returns a Future of a Response.
async fn handle_request(req: Request<Body>) -> Result<Response<Body>, anyhow::Error> {
    match (req.method(), req.uri().path()) {
        // CORS OPTIONS
        (&Method::OPTIONS, "/compute") => Ok(response_build(StatusCode::OK, "")),

        // Serve some instructions at /
        (&Method::GET, "/") => Ok(Response::new(Body::from(
            "Try POSTing data to /compute such as: `curl localhost:8002/compute -XPOST -d '...'`",
        ))),

        (&Method::POST, "/compute") => match compute(req).await {
            Ok(body) => Ok(response_build(StatusCode::OK, &body)),
            Err(err) => Ok(err.into()),
        },

        // Return the 404 Not Found for other routes.
        _ => {
            let mut not_found = Response::default();
            *not_found.status_mut() = StatusCode::NOT_FOUND;
            Ok(not_found)
        }
    }
}

#[derive(Debug)]
enum ComputeError {
    InvalidRequest,
    TaxRateNotAvailable,
    Unexpected(Box<dyn Error + 'static>),
}

impl From<ComputeError> for Response<Body> {
    fn from(value: ComputeError) -> Self {
        let (code, body) = match value {
            ComputeError::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                ErrorResponse::new("invalid request"),
            ),
            ComputeError::TaxRateNotAvailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorResponse::new(
                    "The zip code in the order does not have a corresponding sales tax rate.",
                ),
            ),
            ComputeError::Unexpected(cause) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorResponse::new(format!("{}", cause)),
            ),
        };

        let body = serde_json::to_string_pretty(&body).unwrap();
        response_build(code, &body)
    }
}

#[derive(Serialize)]
struct ErrorResponse {
    status: String,
    message: String,
}

impl ErrorResponse {
    fn new(message: impl ToString) -> Self {
        Self {
            status: "error".to_string(),
            message: message.to_string(),
        }
    }
}

impl From<hyper::Error> for ComputeError {
    fn from(value: hyper::Error) -> Self {
        Self::Unexpected(Box::new(value))
    }
}

impl From<serde_json::Error> for ComputeError {
    fn from(_: serde_json::Error) -> Self {
        Self::InvalidRequest
    }
}

impl From<reqwest::Error> for ComputeError {
    fn from(_: reqwest::Error) -> Self {
        Self::TaxRateNotAvailable
    }
}

impl From<ParseFloatError> for ComputeError {
    fn from(_: ParseFloatError) -> Self {
        Self::TaxRateNotAvailable
    }
}

async fn compute(req: Request<Body>) -> Result<String, ComputeError> {
    let byte_stream = hyper::body::to_bytes(req).await?;
    let mut order: Order = serde_json::from_slice(&byte_stream)?;

    let client = reqwest::Client::new();
    let rate = client
        .post(&*SALES_TAX_RATE_SERVICE)
        .body(order.shipping_zip.clone())
        .send()
        .await?
        .text()
        .await?
        .parse::<f32>()?;

    order.total = order.subtotal * (1.0 + rate);

    let body = serde_json::to_string_pretty(&order)
        .map_err(|err| ComputeError::Unexpected(Box::new(err)))?;

    Ok(body)
}

// CORS headers
fn response_build(status: StatusCode, body: &str) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        .header(
            "Access-Control-Allow-Headers",
            "api,Keep-Alive,User-Agent,Content-Type",
        )
        .body(Body::from(body.to_owned()))
        .unwrap()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([0, 0, 0, 0], 8002));
    let make_svc = make_service_fn(|_| async move {
        Ok::<_, Infallible>(service_fn(move |req| handle_request(req)))
    });
    let server = Server::bind(&addr).serve(make_svc);
    dbg!("Server started on port 8002");
    if let Err(e) = server.await {
        eprintln!("server error: {}", e);
    }
    Ok(())
}
