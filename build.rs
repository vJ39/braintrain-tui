//! rust-embed(#[derive(RustEmbed)])はビルド時にフォルダの中身を読み込むが、
//! Cargoはそのフォルダ自体の変更を追跡しないため、埋め込み元のソースファイルを
//! 変更しない限りassets配下の追加・更新がバイナリに反映されないことがある
//! (画像・音声ファイルを追加してもcargo buildだけでは古いバイナリのままになる不具合の原因)。
//! assetsフォルダの変更を明示的にCargoへ伝え、毎回正しく再ビルドされるようにする

fn main() {
    println!("cargo:rerun-if-changed=assets");
}
