// Development-only localhost fixture. No audio files, recordings, or remote requests.
import http from 'node:http';
const port = 19405;
const rate = 48000;
// Leave time for manual player UI setup and the bounded capture probe.
const fixtureSeconds = 30;
const frames = rate * fixtureSeconds;
// 1730 Hz is an integer bin in the 100 ms probe, away from common voice fundamentals.
const frequencies = [440, 880, 1320, 1730];
const waves = new Map(frequencies.map(hz => {
  const wave = Buffer.alloc(44 + frames * 2);
  wave.write('RIFF', 0); wave.writeUInt32LE(wave.length - 8, 4);
  wave.write('WAVEfmt ', 8); wave.writeUInt32LE(16, 16);
  wave.writeUInt16LE(1, 20); wave.writeUInt16LE(1, 22);
  wave.writeUInt32LE(rate, 24); wave.writeUInt32LE(rate * 2, 28);
  wave.writeUInt16LE(2, 32); wave.writeUInt16LE(16, 34);
  wave.write('data', 36); wave.writeUInt32LE(frames * 2, 40);
  for (let i = 0; i < frames; i++) wave.writeInt16LE(Math.round(32767 / 32 * Math.sin(2 * Math.PI * hz * i / rate)), 44 + i * 2);
  return [hz, wave];
}));
const server = http.createServer((request, response) => {
  const url = new URL(request.url, 'http://127.0.0.1');
  const hz = Number(url.searchParams.get('hz') ?? 440);
  response.setHeader('Cache-Control', 'no-store');
  if (!frequencies.includes(hz)) { response.writeHead(400); response.end(); return; }
  if (url.pathname === '/tone.wav') {
    const wave = waves.get(hz);
    let start = 0, end = wave.length - 1;
    const range = /^bytes=(\d+)-(\d*)$/.exec(request.headers.range ?? '');
    if (request.headers.range && !range) { response.writeHead(416); response.end(); return; }
    if (range) { start = Number(range[1]); end = range[2] ? Number(range[2]) : end; }
    if (start > end || end >= wave.length) { response.writeHead(416); response.end(); return; }
    response.setHeader('Content-Type', 'audio/wav');
    response.setHeader('Accept-Ranges', 'bytes');
    if (range) { response.statusCode = 206; response.setHeader('Content-Range', 'bytes ' + start + '-' + end + '/' + wave.length); }
    response.setHeader('Content-Length', end - start + 1);
    response.end(wave.subarray(start, end + 1));
    return;
  }
  if (url.pathname !== '/') { response.writeHead(404); response.end(); return; }
  response.setHeader('Content-Type', 'text/html; charset=utf-8');
  response.end('<!doctype html><meta charset="utf-8"><title>SubtitleNow 音訊測試 ' + hz + ' Hz</title><h1>SubtitleNow 專用音訊測試</h1><p>本機 RAM 合成音 ' + hz + ' Hz，約 -30 dBFS；不錄音、不更改系統音量。</p><button id="start">開始測試音</button><button id="stop">暫停測試音</button><p id="status">已暫停</p><script>let context, oscillator; document.querySelector("#start").onclick=async()=>{if(!context){context=new AudioContext();oscillator=context.createOscillator();oscillator.frequency.value=' + hz + ';let gain=context.createGain();gain.gain.value=1/32;oscillator.connect(gain).connect(context.destination);oscillator.start()}await context.resume();document.querySelector("#status").textContent="播放中：' + hz + ' Hz"};document.querySelector("#stop").onclick=async()=>{if(context)await context.suspend();document.querySelector("#status").textContent="已暫停"};</script>');
});
server.maxConnections = 8;
server.requestTimeout = 5000;
server.headersTimeout = 5000;
server.listen(port, '127.0.0.1', () => console.log('Local RAM-only test source: http://127.0.0.1:' + port));
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => server.close());
