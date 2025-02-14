use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use clap::Parser;
use geo::algorithm::bounding_rect::BoundingRect;
use geo::{Contains, Point, Polygon};
use rstar::{PointDistance, RTree, RTreeObject, AABB};
use serde_json::Value;
use shapefile::{PolygonRing, Shape};
use std::sync::Arc;

mod record_to_json;

#[derive(Parser, Debug)]
#[command(
    author = "sattosh",
    version = "0.1.0",
    about = "A simple CLI for querying polygons"
)]
struct Args {
    #[arg(short, long)]
    file: Option<String>,
}

// RTree に登録するポリゴン構造体
#[derive(Debug, Clone)]
struct IndexedPolygon {
    polygon: Polygon<f64>,
    // Shapefile の属性情報（DBF の内容）を保持します
    properties: Value,
}

// RTreeObject の実装。各ポリゴンのバウンディングボックスを返します
impl RTreeObject for IndexedPolygon {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        // geo::bounding_rect() は Option を返すので、必ず存在すると仮定して unwrap
        let rect = self.polygon.bounding_rect().unwrap();
        AABB::from_corners([rect.min().x, rect.min().y], [rect.max().x, rect.max().y])
    }
}

// 距離計算用の実装（ここでは envelope の距離を使っています）
impl PointDistance for IndexedPolygon {
    fn distance_2(&self, point: &[f64; 2]) -> f64 {
        self.envelope().distance_2(point)
    }
}

// アプリケーション状態（RTree を共有）
struct AppState {
    rtree: RTree<IndexedPolygon>,
}

// GET /query?lat=...&lon=... でクエリされた位置を検索
async fn query_polygon(
    data: web::Data<Arc<AppState>>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    // クエリパラメータから緯度・経度を取得
    let lat: f64 = query.get("lat").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let lon: f64 = query.get("lon").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let point = Point::new(lon, lat);

    // RTree でバウンディングボックスに含まれる候補を絞り込む
    let candidates = data.rtree.locate_all_at_point(&[point.x(), point.y()]);

    // 候補の中から厳密な点内判定を実施
    for poly in candidates {
        if poly.polygon.contains(&point) {
            // 該当するポリゴンがあれば、その属性情報を JSON として返す
            return HttpResponse::Ok().json(&poly.properties);
        }
    }
    HttpResponse::Ok().json(Value::Null)
}

// Shapefile を読み込み、IndexedPolygon のリストを作成する関数
fn load_polygons_from_shapefile(path: &str) -> Vec<IndexedPolygon> {
    // shapefile::Reader を用いて .shp ファイルを読み込みます（属性は .dbf から取得）
    let mut reader = shapefile::Reader::from_path(path)
        .unwrap_or_else(|e| panic!("Shapefile の読み込みに失敗しました: {}", e));

    let mut polygons = Vec::new();

    // iter_shapes_and_records() で、ジオメトリと属性レコードのペアを取得
    for result in reader.iter_shapes_and_records() {
        let (shape, record) = result.expect("レコードの読み込みエラー");

        match shape {
            // Polygon の場合の処理
            Shape::Polygon(polygon_shape) => {
                let mut poly_list = Vec::new();
                let mut current_polygon: Option<geo::Polygon<f64>> = None;

                // shapefile の Polygon は複数のリングを持つことができる
                for ring in polygon_shape.rings() {
                    match ring {
                        // Outer リングが出た場合は新しいポリゴンを開始
                        PolygonRing::Outer(points) => {
                            // すでに現在のポリゴンがあれば確定してリストに追加
                            if let Some(poly) = current_polygon.take() {
                                poly_list.push(poly);
                            }
                            let exterior_coords = points
                                .iter()
                                .map(|pt| geo::Coord { x: pt.x, y: pt.y })
                                .collect::<Vec<_>>();
                            // 新しいポリゴンを開始（holes は空）
                            current_polygon =
                                Some(geo::Polygon::new(geo::LineString(exterior_coords), vec![]));
                        }
                        // Inner リングの場合は、直前の Outer に付与
                        PolygonRing::Inner(points) => {
                            let interior_coords = points
                                .iter()
                                .map(|pt| geo::Coord { x: pt.x, y: pt.y })
                                .collect::<Vec<_>>();
                            if let Some(poly) = current_polygon.as_mut() {
                                let mut interiors = poly.interiors().to_vec();
                                interiors.push(geo::LineString(interior_coords));
                                *poly = geo::Polygon::new(poly.exterior().clone(), interiors);
                            } else {
                                // Inner リングが最初に来た場合は、どの Outer に属すべきか判断できないので警告を出すか無視する
                                eprintln!("警告: Outer リングが存在しないのに Inner リングが見つかりました");
                            }
                        }
                    }
                }
                // ループ後、現在のポリゴンがあれば追加
                if let Some(poly) = current_polygon.take() {
                    poly_list.push(poly);
                }
                // 属性情報の処理はそのまま
                let properties = record_to_json::record_to_json(&record);
                // マルチポリゴンはPolygonに分割して登録
                for poly in poly_list {
                    polygons.push(IndexedPolygon {
                        polygon: poly,
                        properties: properties.clone(),
                    });
                }
            }
            other_shape => {
                println!("未対応のジオメトリ: {:?}", other_shape.shapetype());
            }
        }
    }
    polygons
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let file_arg = args.file.clone();
    if let Some(file_path) = file_arg {
        println!("指定されたファイルパス: {}", file_path);
    } else {
        println!("ファイルパスが指定されていません");
    }

    let file_path = args.file.unwrap_or_else(|| "data.shp".to_string());

    // Shapefileからポリゴンを読み込み
    let polygons = load_polygons_from_shapefile(&file_path);
    // bulk_load により RTree を一括構築
    let rtree = RTree::bulk_load(polygons);
    let state = Arc::new(AppState { rtree });

    println!("サーバを起動します: http://127.0.0.1:8080");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(state.clone()))
            .route("/query", web::get().to(query_polygon))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
