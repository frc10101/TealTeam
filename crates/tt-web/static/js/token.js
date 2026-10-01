// This device's offline token (C9): who is signed in here, for when the Pi
// cannot be asked.
//
// A signed-in page asks POST /api/auth/token for a token and keeps it in
// localStorage as tt-token:v1 -- which, like agenda.js's list, works over the
// event LAN's plain http. It asks again when there is none, when the one kept
// is someone else's, or when half of its 72 hours are gone. Signing out
// throws it away.
//
// window.ttToken.token() is the token while it lasts, for a sync to send as
// "Authorization: Bearer" once the session has run out (tt_client's Fetch).
// window.ttToken.claims() is what it says -- sub, name, team, roles, dev,
// iat, exp -- read without a key or crypto.subtle: the device trusts its own
// storage for what to show, and the Pi checks the signature on every write.
(function () {
  "use strict";

  var KEY = "tt-token:v1";
  var HEADER = "v4.public.";
  var SIGNATURE_BYTES = 64;
  var HALF_LIFE_MS = 36 * 60 * 60 * 1000;

  var script = document.currentScript;
  var user = script ? Number(script.dataset.user) : 0;

  function read() {
    try {
      return window.localStorage.getItem(KEY);
    } catch (e) {
      return null;
    }
  }
  function write(token) {
    try {
      window.localStorage.setItem(KEY, token);
    } catch (e) {
      // Full, or private browsing: the session still works while it lasts.
    }
  }
  function remove() {
    try {
      window.localStorage.removeItem(KEY);
    } catch (e) {}
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

  window.ttToken = {
    token: function () {
      var token = read();
      return left(claimsOf(token)) > 0 ? token : null;
    },
    claims: function () {
      var claims = claimsOf(read());
      return left(claims) > 0 ? claims : null;
    },
  };

  document.addEventListener("submit", function (event) {
    var action = event.target.getAttribute("action") || "";
    if (action.indexOf("/api/auth/logout") === 0) remove();
  });

  // The offline shell knows no one, and only reads.
  if (!user) return;
  var claims = claimsOf(read());
  if (claims && Number(claims.sub) === user && left(claims) > HALF_LIFE_MS) return;
  if (claims && Number(claims.sub) !== user) remove();
  // Failure is expected at an event; the next page asks again.
  fetch("/api/auth/token", { method: "POST", credentials: "same-origin", redirect: "manual" })
    .then(function (response) {
      return response.ok ? response.json() : null;
    })
    .then(function (issued) {
      if (issued && issued.token) write(issued.token);
    })
    .catch(function () {});
})();
