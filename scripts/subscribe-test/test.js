const WebSocket = require('websocket').w3cwebsocket;

var subscriptions = {};

const ws = new WebSocket('ws://my-coin-server:8340');
ws.addEventListener('open', () => {
  console.log('Sending request');

  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: "forks",
    method: "subscribe_forks",
    params: null,
  }));

  ws.send(JSON.stringify({
    jsonrpc: "2.0",
    id: "validation_checks",
    method: "validation_checks",
    params: null,
  }));
});


process.on('SIGINT', function () {
    console.log("Cancelling subscriptions");
    for ( const [key, id] of Object.entries(subscriptions)) {
        console.log(`Unsubscribe ${key} with id ${id}`);

        const method = `unsubscribe_${key}`;

        ws.send(JSON.stringify({
            jsonrpc: "2.0",
            id: method,
            method: method,
            params: { id: id },
        }));
    }
    process.exit();
});


ws.addEventListener('message', (message) => {
  const obj = JSON.parse(message.data);

  if (obj.id !== undefined) {
      console.log('Subscription id: ', obj.result);
      subscriptions[obj.id] = obj.result;
  } else {
      if (obj.method == "forks") {
          console.log(`Got forks method: ${obj.params}`);
      } else if (obj.method == "validation_checks") {
          console.log(`Got checks method: ${obj.params}`);
      }
  }
});

console.log('Starting');
