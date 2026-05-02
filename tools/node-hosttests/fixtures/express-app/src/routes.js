exports.install = function(app) {
  app.get('/hello', function(req, res) {
    res.type('text/plain').send('hello express:' + req.method + ':' + req.path);
  });

  app.get('/json', function(req, res) {
    res.status(201).json({
      ok: true,
      path: req.path,
      middleware: req.middlewareTag
    });
  });

  app.get('/headers', function(req, res) {
    res.set('x-powered-by', 'anyos-node').send(req.get('host'));
  });
};
