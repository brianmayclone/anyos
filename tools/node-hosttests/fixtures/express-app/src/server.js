const express = require('express');
const routes = require('./routes');

function createApp() {
  const app = express();

  app.use(function(req, res, next) {
    req.middlewareTag = 'seen';
    next();
  });

  routes.install(app);
  return app;
}

exports.createApp = createApp;

exports.start = function(port) {
  return createApp().listen(port);
};
