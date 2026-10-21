// This device's offline token (C9): who is signed in here, for when the Pi
// cannot be asked.
//
// A signed-in page asks POST /api/auth/token for a token and its device key
// and keeps both in localStorage as tt-token:v2 -- which, like agenda.js's
// list, works over the event LAN's plain http. It asks again when there is
// none, when the one kept is someone else's, or when half of its 72 hours
// are gone. Signing in or out throws it away, so a scout signed out
// everywhere (T1) gets a fresh one with their next sign-in. A tt-token:v1
// (a token with no key, from before T1) is thrown away and asked for again.
//
// window.ttToken.token() is the credential while it lasts -- the token, "~",
// the key -- for a sync to send as "Authorization: Bearer" once the session
// has run out (outbox.js, tt_client's Fetch). The token alone is no use to
// the Pi. window.ttToken.claims() is what it says -- sub, name, team, roles,
// dev, ep, iat, exp -- read without a key or crypto.subtle: the device
// trusts its own storage for what to show, and the Pi checks every write.
// window.ttToken.key() is the device key's bytes, for the QR handoff's MACs
// (handoff.js); window.ttToken.holder() is who the handoff names instead of
// sending the token.
(function () {
  "use strict";

  var KEY = "tt-token:v2";
  var OLD_KEY = "tt-token:v1";
  var HEADER = "v4.public.";
  var SIGNATURE_BYTES = 64;
  var HALF_LIFE_MS = 36 * 60 * 60 * 1000;

  var script = document.currentScript;
  var user = script ? Number(script.dataset.user) : 0;

  // {token, key}, or null.
  function read() {
    try {
      var kept = JSON.parse(window.localStorage.getItem(KEY) || "null");
      return kept && typeof kept.token === "string" && typeof kept.key === "string" ? kept : null;
    } catch (e) {
      return null;
    }
  }
  function write(issued) {
    try {
      window.localStorage.setItem(KEY, JSON.stringify({ token: issued.token, key: issued.key }));
    } catch (e) {
      // Full, or private browsing: the session still works while it lasts.
    }
  }
  function remove() {
    try {
      window.localStorage.removeItem(KEY);
    } catch (e) {}
  }
  try {
    window.localStorage.removeItem(OLD_KEY);
  } catch (e) {}

  function keyBytes(text) {
    try {
      var binary = window.atob(text.replace(/-/g, "+").replace(/_/g, "/"));
      if (binary.length !== 32) return null;
      var bytes = new Uint8Array(32);
      for (var i = 0; i < 32; i++) bytes[i] = binary.charCodeAt(i);
      return bytes;
    } catch (e) {
      return null;
    }
  }

  // The JSON before the signature, in the token's unpadded base64url.
  function claimsOf(token) {
    if (!token || token.indexOf(HEADER) !== 0) return null;
    try {
      var body = token.slice(HEADER.length).split(".")[0];
      var binary = window.atob(body.replace(/-/g, "+").replace(/_/g, "/"));
      var bytes = new Uint8Array(binary.length);
      for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      var json = new TextDecoder().decode(bytes.subarray(0, bytes.length - SIGNATURE_BYTES));
      return JSON.parse(json);
    } catch (e) {
      return null;
    }
  }
  function left(claims) {
    return claims ? Date.parse(claims.exp) - Date.now() : -1;
  }

  // What is kept, while its token lasts.
  function live() {
    var kept = read();
    var claims = kept && claimsOf(kept.token);
    return left(claims) > 0 && keyBytes(kept.key) ? { kept: kept, claims: claims } : null;
  }

  window.ttToken = {
    token: function () {
      var now = live();
      return now ? now.kept.token + "~" + now.kept.key : null;
    },
    claims: function () {
      var now = live();
      return now ? now.claims : null;
    },
    key: function () {
      var now = live();
      return now ? keyBytes(now.kept.key) : null;
    },
    // tt_core::token::Holder: the claims that name the token, and no more.
    holder: function () {
      var now = live();
      if (!now) return null;
      var c = now.claims;
      return { sub: c.sub, dev: c.dev, ep: c.ep, iat: c.iat };
    },
  };

  document.addEventListener("submit", function (event) {
    var action = event.target.getAttribute("action") || "";
    if (action.indexOf("/api/auth/logout") === 0 || action.indexOf("/api/auth/login") === 0) remove();
  });

  // The offline shell knows no one, and only reads.
  if (!user) return;
  var kept = read();
  var claims = kept && claimsOf(kept.token);
  if (claims && Number(claims.sub) === user && left(claims) > HALF_LIFE_MS && keyBytes(kept.key)) return;
  if (claims && Number(claims.sub) !== user) remove();
  // Failure is expected at an event; the next page asks again.
  fetch("/api/auth/token", { method: "POST", credentials: "same-origin", redirect: "manual" })
    .then(function (response) {
      return response.ok ? response.json() : null;
    })
    .then(function (issued) {
      if (issued && issued.token && issued.key) write(issued);
    })
    .catch(function () {});
})();
