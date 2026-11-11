// Browser side of examples/websocket_demo/server.tnt.
const url = (path) => (location.protocol === "https:" ? "wss://" : "ws://") + location.host + path;
const log = (id, line) => {
  const el = document.getElementById(id);
  if (el) el.textContent += line + "\n";
};

// Reconnect when the server closes: lifetime limit (1001), development
// reload (1012) and overload (1013) are all normal. 1008 means the client
// sent too much; don't hammer the server.
function connect(path, logId) {
  const ws = new WebSocket(url(path));
  ws.onmessage = (e) => log(logId, e.data);
  ws.onclose = (e) => {
    log(logId, "closed " + e.code + (e.reason ? " " + e.reason : ""));
    if (e.code !== 1008) setTimeout(() => connect(path, logId), 1000);
  };
  return ws;
}

let echo = connect("/ws/echo", "echo-log");
document.getElementById("echo-send").onclick = () => {
  if (echo.readyState !== WebSocket.OPEN) echo = connect("/ws/echo", "echo-log");
  else echo.send(document.getElementById("echo-text").value);
};

if (document.getElementById("feed-log")) {
  connect("/ws/feed", "feed-log");
  document.getElementById("announce").onsubmit = async (e) => {
    e.preventDefault();
    await fetch("/announce", { method: "POST", body: new URLSearchParams(new FormData(e.target)) });
    e.target.text.value = "";
  };
}
