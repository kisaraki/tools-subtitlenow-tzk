# ADR 0021 — 內容辨識結果在 owner 邊界取消、過期與清除

日期：2026-10-09。狀態：Phase 6 第八批採用。

正式 Live 接線後，獨立 ResolverSession 的公開介面仍可能在 task 尚未再次輪詢時交付舊 Ready：取消只發通知，poll 原先僅核對 epoch；未讀結果不重新檢查時間；invalidate 雖更新 input，output 仍持有舊歌曲／電影 metadata。只在 LiveContent 再次過濾，不能保證其他 caller 的同一契約，亦不能回收共享槽位中隱藏的舊結果。

Session 現持有 output sender。替換視窗、invalidate 與新增 stop 由 owner 同步清除共享結果；每個 input 視窗持有獨立 request token，替換／重設／停止會立即通知該要求。外部 cancellation token 仍是取消通知：poll／next 會轉成空的 Stopped，task 的取消分支或 Drop 回收 future；沒有宣稱可以強行釋放尚未輪詢 future 或 caller 仍持有的 Arc。

Ready／Resolving 每次取用重新驗證原 stamp 的 Session／generation／既有五秒 freshness。過期會取消要求、清除共享音訊／metadata、推進 reset epoch 並回報 Failed(Stale)，不是以讀取時間重新延長 lease。next 同時等待 output、取消與原始時間的 deadline，即使沒有新音訊或沒有輪詢 task 也能處理過期與終止。錯誤／停止狀態只保留真實 WindowStamp 與空 Resolution；未曾有有效視窗時不捏造 stamp。

input replacement 與 output 清除在同一短暫 input write gate 中完成；task publication 使用相同 input read gate，核對 epoch、最新視窗 sequence 及取消狀態才寫 output。這同時避免舊回覆在清除後填回槽位，以及新有效回覆在 input 已可見後又被 owner 的延遲 clear 擦掉。鎖定順序固定 input → output，PCM／metadata 的 allocation 與舊值 Drop 都在 gate 外，沒有 await、網路、FIR 或 capture callback。

提交新視窗時在 write gate 再次檢查取消，防止 task Drop 與 admission 競爭後仍留下 pending PCM。epoch 溢位停止而不回繞。task 看見 reset 只清除私有投票，不再以舊 current stamp 重發 owner 的通知；Drop 優先使用佇列中的真實最新 stamp，且不覆蓋同 epoch 已存在的 Stopped，保留較新時間及單一終態通知。

LiveResolver.stop 使用新的同步 Session.stop，再 Drop provider future、停止／join 音訊 CPU lane。原有 STT／翻譯、共用 TranscriptTimeline、供應商選擇與固定金鑰介面不變。本批沒有新的 thread、無界 queue、unsafe、相依套件、正式內容服務或網路要求；資料仍只在 Session RAM。

二十項新增合成回歸測試覆蓋 task 未再次輪詢時的取消、替換、重設、結果 Weak 回收、Ready／Resolving 已讀／未讀過期、next 喚醒、stop／Drop／真實 stamp、epoch 溢位，以及三組各二百次跨執行緒競爭：舊發布與清除、新有效發布與替換、task Drop 與新提交。有界 channel 使用逾時；不把這些測試當作真實內容識別、播放器相容性或效能量測。