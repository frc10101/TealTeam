// Reading QR codes with this device's camera (S13).
//
// BarcodeDetector where the browser has one that reads QR codes: native,
// fast, nothing to load. Chrome has it on Android, ChromeOS, and macOS, and
// only in a secure context. Everywhere else, zxing-wasm (static/vendor/zxing,
// about 1 MB), loaded when first needed -- or, with preload(), as soon as
// the page is up, so a tablet with no server can still scan.
//
// Streaming the camera needs a secure context too (https, or the device
// itself). Over the event LAN's plain http there is no stream, only a photo:
// <input type=file capture> opens the camera app, and the picture is read
// the same way.
//
// window.ttScan:
//   preload()               fetch the reader now, while the server is there
//   canStream()             whether this page can stream the camera
//   stream(video, onTexts)  -> Promise<{stop()}>; onTexts([text]) for every
//                           look that found a code, several times a second
//   photo(file)             -> Promise<[text]>
(function () {
  "use strict";

  var BASE = "/static/vendor/zxing/";
  var LOOKS_PER_SECOND = 6;
  // A phone's photo is 12 MP; a code that fills a fifth of it is still
  // several pixels a module at this size.
  var MAX_PHOTO_SIDE = 2000;
  var ZXING_OPTIONS = { formats: ["QRCode"], tryHarder: true, maxNumberOfSymbols: 4 };

  var script = null;
  var wasm = null;
  var reader = null;

  function loadScript() {
    if (window.ZXingWASM) return Promise.resolve();
    if (!script) {
      script = new Promise(function (resolve, reject) {
        var el = document.createElement("script");
        el.src = BASE + "zxing-reader.js";
        el.onload = function () {
          resolve();
        };
        el.onerror = function () {
          script = null;
          reject(new Error("The code reader could not be loaded. Reload the page while the server is in reach."));
        };
        document.head.appendChild(el);
      });
    }
    return script;
  }

  function loadWasm() {
    if (!wasm) {
      wasm = fetch(BASE + "zxing_reader.wasm", { credentials: "same-origin" })
        .then(function (response) {
          if (!response.ok) throw new Error(String(response.status));
          return response.arrayBuffer();
        })
        .catch(function () {
          wasm = null;
          throw new Error("The code reader could not be loaded. Reload the page while the server is in reach.");
        });
    }
    return wasm;
  }

  function native() {
    if (!("BarcodeDetector" in window)) return Promise.resolve(null);
    return Promise.resolve(window.BarcodeDetector.getSupportedFormats())
      .then(function (formats) {
        if (formats.indexOf("qr_code") < 0) return null;
        var detector = new window.BarcodeDetector({ formats: ["qr_code"] });
        return function (source) {
          return detector.detect(source).then(function (found) {
            return found.map(function (b) {
              return b.rawValue;
            });
          });
        };
      })
      .catch(function () {
        return null;
      });
  }

  function zxing() {
    return Promise.all([loadScript(), loadWasm()]).then(function (loaded) {
      window.ZXingWASM.setZXingModuleOverrides({
        // Never the CDN it would otherwise ask (U6).
        locateFile: function (path) {
          return BASE + path;
        },
        wasmBinary: loaded[1],
      });
      var canvas = document.createElement("canvas");
      var context = canvas.getContext("2d", { willReadFrequently: true });
      return function (source) {
        var w = source.videoWidth || source.width;
        var h = source.videoHeight || source.height;
        if (!w || !h) return Promise.resolve([]);
        canvas.width = w;
        canvas.height = h;
        context.drawImage(source, 0, 0, w, h);
        return window.ZXingWASM.readBarcodes(context.getImageData(0, 0, w, h), ZXING_OPTIONS).then(function (found) {
          return found
            .filter(function (b) {
              return b.isValid !== false && b.text;
            })
            .map(function (b) {
              return b.text;
            });
        });
      };
    });
  }

  // The reader this browser has: a function from an image to the texts in it.
  function read() {
    if (!reader) {
      reader = native().then(function (found) {
        return found || zxing();
      });
      reader.catch(function () {
        reader = null;
      });
    }
    return reader;
  }

  function preload() {
    if ("BarcodeDetector" in window) {
      native().then(function (found) {
        if (!found) Promise.all([loadScript(), loadWasm()]).catch(function () {});
      });
    } else {
      Promise.all([loadScript(), loadWasm()]).catch(function () {});
    }
  }

  function canStream() {
    return !!(navigator.mediaDevices && navigator.mediaDevices.getUserMedia);
  }

  function stream(video, onTexts) {
    if (!canStream()) return Promise.reject(new Error("This browser cannot stream the camera here."));
    return Promise.all([
      read(),
      navigator.mediaDevices.getUserMedia({
        audio: false,
        video: { facingMode: { ideal: "environment" }, width: { ideal: 1280 }, height: { ideal: 720 } },
      }),
    ]).then(function (both) {
      var look = both[0];
      var media = both[1];
      var stopped = false;
      video.srcObject = media;
      video.setAttribute("playsinline", "");
      video.muted = true;
      var playing = video.play();
      function tick() {
        if (stopped) return;
        var started = Date.now();
        var next = function () {
          setTimeout(tick, Math.max(0, 1000 / LOOKS_PER_SECOND - (Date.now() - started)));
        };
        if (video.readyState < 2) return next();
        look(video)
          .then(function (texts) {
            if (texts.length && !stopped) onTexts(texts);
          })
          .catch(function () {})
          .then(next);
      }
      return Promise.resolve(playing).then(function () {
        tick();
        return {
          stop: function () {
            stopped = true;
            media.getTracks().forEach(function (t) {
              t.stop();
            });
            video.srcObject = null;
          },
        };
      });
    });
  }

  function photo(file) {
    return Promise.all([read(), createImageBitmap(file)]).then(function (both) {
      var look = both[0];
      var bitmap = both[1];
      var scale = Math.min(1, MAX_PHOTO_SIDE / Math.max(bitmap.width, bitmap.height));
      var canvas = document.createElement("canvas");
      canvas.width = Math.round(bitmap.width * scale);
      canvas.height = Math.round(bitmap.height * scale);
      canvas.getContext("2d").drawImage(bitmap, 0, 0, canvas.width, canvas.height);
      if (bitmap.close) bitmap.close();
      return look(canvas);
    });
  }

  window.ttScan = { preload: preload, canStream: canStream, stream: stream, photo: photo };
})();
