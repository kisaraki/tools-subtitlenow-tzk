# 相依套件授權紀錄

## 本機文字語言判定

Phase 11 Auto 新增 whatlang 0.18.0，直接版本精確固定、額外 features 停用，全部解析版本記於 Cargo.lock；使用全部內建語言模型後才篩選五語，不下載執行時模型、不收集使用者內容。新增解析套件宣告如下，五份套件皆沒有 build script，也沒有原生 DLL 或系統安裝。

| 套件 | 版本 | 宣告授權 |
| --- | --- | --- |
| whatlang | 0.18.0 | MIT |
| hashbrown | 0.15.5 | MIT OR Apache-2.0 |
| foldhash | 0.1.5 | Zlib |
| equivalent | 1.0.3 | Apache-2.0 OR MIT |
| allocator-api2 | 0.2.21 | MIT OR Apache-2.0 |

whatlang 的 crate 封包未含獨立 LICENSE，已保留 [同版本上游原文](https://raw.githubusercontent.com/greyblake/whatlang-rs/v0.18.0/LICENSE) 於 whatlang-0.18.0-LICENSE-MIT.txt；其他四份套件隨附原始通知亦依版本保留在本目錄。來源：[whatlang v0.18.0](https://github.com/greyblake/whatlang-rs/tree/v0.18.0)、[版本 manifest](https://docs.rs/crate/whatlang/0.18.0/source/Cargo.toml)。這些是一般統計模型／程式庫授權，不是音訊、字幕或辨識內容快取。

## 臺灣字詞轉換

本程式使用 zhconv 0.4.2 的程式庫（MIT／Apache-2.0）與 OpenCC 轉換字詞表（Apache-2.0）。啟用 opencc、opencc-twp、compress，停用預設 features，未啟用 MediaWiki 的 GPL 字詞表。

來源：https://github.com/Gowee/zhconv-rs ，https://github.com/BYVoid/OpenCC 。zhconv 的版本及 feature 由 Cargo.toml／Cargo.lock 固定。原始授權文字保留於同資料夾的 zhconv-MIT.txt 與 OpenCC-Apache-2.0.txt。這些是一般字詞轉換資料，不包含任何使用者音訊、逐字稿、翻譯或 Session 詞彙。

## 本機音訊指紋

新增 rusty-chromaprint 0.3.0，直接版本固定在 Cargo.toml，全部解析版本固定在 Cargo.lock。它是 Rust port，這次建置未加入原生 Chromaprint／FFmpeg DLL，也不安裝額外 Windows 元件。以下列出實際下載套件 Cargo.toml 的 license 欄位。

| 套件 | 本次解析版本 | 套件宣告 |
| --- | --- | --- |
| rusty-chromaprint | 0.3.0 | MIT |
| rubato | 0.16.2 | MIT |
| rustfft | 6.4.1 | MIT OR Apache-2.0 |
| realfft | 3.5.0 | MIT |
| num-complex | 0.4.6 | MIT OR Apache-2.0 |
| num-integer | 0.1.47 | MIT OR Apache-2.0 |
| num-traits | 0.2.19 | MIT OR Apache-2.0 |
| transpose | 0.2.3 | MIT OR Apache-2.0 |
| primal-check | 0.3.4 | MIT OR Apache-2.0 |
| strength_reduce | 0.2.4 | MIT OR Apache-2.0 |
| autocfg | 1.5.1 | Apache-2.0 OR MIT |

同資料夾以「套件-版本-LICENSE」保留各下載套件隨附的原始通知。rusty-chromaprint 0.3.0 與 realfft 3.5.0 套件未附獨立 LICENSE 檔，保留上述實際 manifest 宣告與來源識別，沒有自行補造作者／年份。另保留 Chromaprint-upstream-LICENSE.md：原生上游文件區分自身 MIT 程式與包含的 FFmpeg LGPL 部分，本專案沒有建置該原生上游組合。

來源：[rusty-chromaprint](https://github.com/darksv/rusty-chromaprint)、[版本 manifest](https://docs.rs/crate/rusty-chromaprint/0.3.0/source/Cargo.toml)、[Chromaprint 上游授權](https://github.com/acoustid/chromaprint/blob/master/LICENSE.md)、[Rubato](https://github.com/HEnquist/rubato)、[RustFFT](https://github.com/ejmahler/RustFFT)、[RealFFT](https://github.com/HEnquist/realfft)。本目錄只保存程式庫授權文字；不包含音訊、指紋、逐字稿、翻譯或目錄查詢結果。

## HTTP 日期

Phase 6 歌詞 adapter 新增 httpdate 1.0.3，直接版本與 Cargo.lock 均固定，用於解析 Retry-After 的 HTTP 日期。套件宣告 MIT OR Apache-2.0；原始通知保留於 httpdate-1.0.3-LICENSE-MIT 與 httpdate-1.0.3-LICENSE-APACHE。這個套件不加入網路服務、音訊或內容快取。

來源：[httpdate 1.0.3](https://docs.rs/crate/httpdate/1.0.3)、[套件來源](https://github.com/pyfisch/httpdate)。
