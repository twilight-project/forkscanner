const WebSocket = require('websocket').w3cwebsocket;

const ws = new WebSocket('ws://my-coin-server:8340');
ws.addEventListener('open', () => {
  console.log('Sending request');

  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: 1,
    method: "validation_checks",
    params: [20],
  }));
});

ws.addEventListener('message', (message) => {
  console.log('Received: ', message.data);
});

console.log('Starting');
