// QR transfer's format in the browser (S13): the same frames tt_core::qr
// makes and reads, for a tablet with no Pi to ask.
//
//   TT1:<kind>:<crc32 of the compressed bytes, 8 hex>:<i>/<n>:<base45>
//
// zlib by CompressionStream where the browser has it, else zlib's "stored"
// blocks, which any inflater reads: bigger, never wrong. Reading a transfer
// needs DecompressionStream (Chrome 80, Safari 16.4).
//
// window.ttQr:
//   frames(kind, bytes) -> Promise<[text]>
//   new Assembler(): add(text) -> {kind, have, count, fresh}, missing(),
//     finish() -> Promise<{kind, bytes}>; add() throws an Error with a
//     sentence for a code that is not one of ours or is damaged, and one
//     with .other = true for a code of another transfer.
//   draw(text) -> <svg> (needs vendor/qrcode-generator/qrcode.js)
//   cycle(container, ms) -> {pause(), resume(), next(), paused()}: shows the
//     container's children one at a time, in turn.
(function () {
  "use strict";

  var PREFIX = "TT1:";
  var FRAME_BYTES = 400;
  var MAX_FRAMES = 99;
  var QUIET_ZONE = 4;
  var BASE45 = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
  var KINDS = { H: true, R: true };
  var SVG = "http://www.w3.org/2000/svg";

  function base45(bytes) {
    var out = "";
    for (var i = 0; i < bytes.length; i += 2) {
      if (i + 1 < bytes.length) {
        var n = bytes[i] * 256 + bytes[i + 1];
        out += BASE45[n % 45] + BASE45[Math.floor(n / 45) % 45] + BASE45[Math.floor(n / 2025)];
      } else {
        out += BASE45[bytes[i] % 45] + BASE45[Math.floor(bytes[i] / 45)];
      }
    }
    return out;
  }

  function unbase45(text) {
    var out = [];
    for (var i = 0; i < text.length; i += 3) {
      var group = text.slice(i, i + 3);
      var v = [];
      for (var j = 0; j < group.length; j++) {
        var d = BASE45.indexOf(group[j]);
        if (d < 0) return null;
        v.push(d);
      }
      if (v.length === 3) {
        var n = v[0] + v[1] * 45 + v[2] * 2025;
        if (n > 0xffff) return null;
        out.push(n >> 8, n & 0xff);
      } else if (v.length === 2) {
        var m = v[0] + v[1] * 45;
        if (m > 0xff) return null;
        out.push(m);
      } else {
        return null;
      }
    }
    return new Uint8Array(out);
  }

  function crc32(bytes) {
    var crc = -1;
    for (var i = 0; i < bytes.length; i++) {
      crc ^= bytes[i];
      for (var k = 0; k < 8; k++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
    }
    return (crc ^ -1) >>> 0;
  }

  function adler32(bytes) {
    var a = 1, b = 0;
    for (var i = 0; i < bytes.length; i++) {
      a = (a + bytes[i]) % 65521;
      b = (b + a) % 65521;
    }
    return ((b << 16) | a) >>> 0;
  }

  // zlib with no compression: a header, stored blocks, the checksum.
  function stored(bytes) {
    var out = [0x78, 0x01];
    for (var at = 0; at < bytes.length || at === 0; at += 65535) {
      var block = bytes.subarray(at, at + 65535);
      var last = at + 65535 >= bytes.length ? 1 : 0;
      out.push(last, block.length & 0xff, block.length >> 8, ~block.length & 0xff, (~block.length >> 8) & 0xff);
      for (var i = 0; i < block.length; i++) out.push(block[i]);
      if (last) break;
    }
    var sum = adler32(bytes);
    out.push(sum >>> 24, (sum >>> 16) & 0xff, (sum >>> 8) & 0xff, sum & 0xff);
    return new Uint8Array(out);
  }

  function through(stream, bytes) {
    var piped = new Blob([bytes]).stream().pipeThrough(stream);
    return new Response(piped).arrayBuffer().then(function (buffer) {
      return new Uint8Array(buffer);
    });
  }

  function deflate(bytes) {
    if (typeof CompressionStream !== "function") return Promise.resolve(stored(bytes));
    return through(new CompressionStream("deflate"), bytes).catch(function () {
      return stored(bytes);
    });
  }

  function inflate(bytes) {
    if (typeof DecompressionStream !== "function") {
      return Promise.reject(new Error("This browser cannot read these codes. Update it, or use another device."));
    }
    return through(new DecompressionStream("deflate"), bytes).catch(function () {
      throw new Error("The codes do not add up. Scan them again.");
    });
  }

  function hex8(n) {
    return ("0000000" + n.toString(16).toUpperCase()).slice(-8);
  }

  function frames(kind, bytes) {
    return deflate(bytes).then(function (packed) {
      var count = Math.max(1, Math.ceil(packed.length / FRAME_BYTES));
      if (count > MAX_FRAMES) throw new Error("Too much for one transfer: at most " + MAX_FRAMES + " codes.");
      var id = hex8(crc32(packed));
      var out = [];
      for (var i = 0; i < count; i++) {
        var chunk = packed.subarray(i * FRAME_BYTES, (i + 1) * FRAME_BYTES);
        out.push(PREFIX + kind + ":" + id + ":" + (i + 1) + "/" + count + ":" + base45(chunk));
      }
      return out;
    });
  }

  function damaged(why) {
    return new Error("That code is damaged (" + why + "). Hold it steady and try again.");
  }

  // A frame's header and data, or an Error saying what it is instead.
  function parse(text) {
    text = String(text).trim();
    if (text.indexOf(PREFIX) !== 0) {
      var tag = text.split(":")[0];
      if (/^TT\d+$/.test(tag)) return new Error("That code was made by another version of TealTeam (" + tag + ").");
      return new Error("That is not a TealTeam code.");
    }
    var parts = text.slice(PREFIX.length).split(":");
    if (parts.length < 4) return damaged("a part of its header is missing");
    var kind = parts[0];
    var id = parts[1];
    var place = /^(\d+)\/(\d+)$/.exec(parts[2]);
    var data = parts.slice(3).join(":");
    if (!KINDS[kind]) return damaged("an unknown kind");
    if (!/^[0-9A-F]{8}$/.test(id)) return damaged("its id");
    if (!place) return damaged("its place");
    var index = Number(place[1]), count = Number(place[2]);
    if (count < 1 || count > MAX_FRAMES || index < 1 || index > count) return damaged("its place");
    return { kind: kind, id: id, index: index, count: count, data: data };
  }

  function Assembler() {
    this.reset();
  }
  Assembler.prototype.reset = function () {
    this.kind = null;
    this.id = null;
    this.chunks = null;
  };
  Assembler.prototype.progress = function (fresh) {
    if (!this.chunks) return { kind: null, have: 0, count: 0, fresh: false };
    var have = this.chunks.filter(Boolean).length;
    return { kind: this.kind, have: have, count: this.chunks.length, fresh: !!fresh };
  };
  Assembler.prototype.add = function (text) {
    var frame = parse(text);
    if (frame instanceof Error) throw frame;
    if (!this.chunks) {
      this.kind = frame.kind;
      this.id = frame.id;
      this.chunks = new Array(frame.count).fill(null);
    }
    if (frame.id !== this.id || frame.kind !== this.kind) {
      var other = new Error("That code belongs to another transfer.");
      other.other = true;
      throw other;
    }
    if (this.chunks.length !== frame.count) throw damaged("two parts disagree on how many there are");
    if (this.chunks[frame.index - 1]) return this.progress(false);
    var bytes = unbase45(frame.data);
    var last = frame.index === frame.count;
    if (!bytes || !bytes.length || bytes.length > FRAME_BYTES || (!last && bytes.length !== FRAME_BYTES)) {
      throw damaged("its data");
    }
    this.chunks[frame.index - 1] = bytes;
    return this.progress(true);
  };
  Assembler.prototype.missing = function () {
    var out = [];
    (this.chunks || []).forEach(function (c, i) {
      if (!c) out.push(i + 1);
    });
    return out;
  };
  Assembler.prototype.finish = function () {
    var kind = this.kind, id = this.id, chunks = this.chunks;
    if (!chunks || chunks.some(function (c) { return !c; })) {
      return Promise.reject(new Error("Some parts are still missing."));
    }
    var length = chunks.reduce(function (n, c) { return n + c.length; }, 0);
    var packed = new Uint8Array(length);
    var at = 0;
    chunks.forEach(function (c) {
      packed.set(c, at);
      at += c.length;
    });
    if (hex8(crc32(packed)) !== id) return Promise.reject(new Error("The codes do not add up. Scan them again."));
    return inflate(packed).then(function (bytes) {
      return { kind: kind, bytes: bytes };
    });
  };

  // One code as SVG, drawn as tt_core::qr::symbol draws it: medium error
  // correction, alphanumeric mode, a four-module quiet zone.
  function draw(text) {
    var code = window.qrcode(0, "M");
    code.addData(text, "Alphanumeric");
    code.make();
    var n = code.getModuleCount();
    var size = n + 2 * QUIET_ZONE;
    var path = "";
    for (var y = 0; y < n; y++) {
      for (var x = 0; x < n; ) {
        if (!code.isDark(y, x)) {
          x++;
          continue;
        }
        var start = x;
        while (x < n && code.isDark(y, x)) x++;
        path += "M" + (start + QUIET_ZONE) + "," + (y + QUIET_ZONE) + "h" + (x - start) + "v1h-" + (x - start) + "z";
      }
    }
    var svg = document.createElementNS(SVG, "svg");
    svg.setAttribute("class", "qr");
    svg.setAttribute("viewBox", "0 0 " + size + " " + size);
    svg.setAttribute("shape-rendering", "crispEdges");
    svg.setAttribute("role", "img");
    var rect = document.createElementNS(SVG, "rect");
    rect.setAttribute("width", size);
    rect.setAttribute("height", size);
    rect.setAttribute("fill", "#fff");
    var dark = document.createElementNS(SVG, "path");
    dark.setAttribute("d", path);
    dark.setAttribute("fill", "#000");
    svg.appendChild(rect);
    svg.appendChild(dark);
    svg.dataset.text = text;
    return svg;
  }

  function cycle(container, ms, onShow) {
    var items = Array.prototype.slice.call(container.children);
    var at = 0;
    var timer = null;
    function show(i) {
      at = i % items.length;
      items.forEach(function (el, j) {
        el.hidden = j !== at;
      });
      if (onShow) onShow(at, items.length);
    }
    function resume() {
      clearInterval(timer);
      if (items.length > 1) {
        timer = setInterval(function () {
          show(at + 1);
        }, ms);
      }
    }
    show(0);
    resume();
    return {
      pause: function () {
        clearInterval(timer);
        timer = null;
      },
      resume: resume,
      next: function () {
        show(at + 1);
      },
      paused: function () {
        return timer === null;
      },
      stop: function () {
        clearInterval(timer);
      },
    };
  }

  window.ttQr = { frames: frames, Assembler: Assembler, draw: draw, cycle: cycle, parse: parse };

  // A page's own codes, drawn by the server, shown in turn.
  document.querySelectorAll("[data-qr-cycle]").forEach(function (el) {
    cycle(el, 700);
  });
})();
