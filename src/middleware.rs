use std::sync::Arc;
use error::Error;

use super::{Request, Response, IronResult};

/// `Handler`s are responsible for handling requests by creating Responses from Requests.
///
/// By default, bare functions and variants of Chain implement `Handler`.
///
/// `Handler`s are allowed to return errors, and if they do, their `catch` method is called and the
/// error is propagated to `AfterMiddleware`.
pub trait Handler: Send + Sync {
    /// Produce a `Response` from a Request, with the possibility of error.
    ///
    /// If this returns an Err, `catch` is called with the error.
    fn call(&self, &mut Request) -> IronResult<Response>;

    /// If `Handler`'s call method produces an Err, then this method is called
    /// to produce a `Response` and possibly handle the error.
    ///
    /// If the passed-in error is not handled, it should be returned as the second
    /// item in the returned tuple. If it is handled, then `Ok(())` can be returned
    /// instead to indicate that all is good with the Response.
    fn catch(&self, &mut Request, Box<Error>) -> (Response, IronResult<()>);
}

/// `BeforeMiddleware` are fired before a `Handler` is called inside of a Chain.
///
/// `BeforeMiddleware` are responsible for doing request pre-processing that requires
/// the ability to change control-flow, such as authorization middleware, or for editing
/// the request by modifying the headers.
///
/// `BeforeMiddleware` only have access to the Request, if you need to modify or read a Response,
/// you will need `AfterMiddleware`.
pub trait BeforeMiddleware: Send + Sync {
    /// Do whatever work this middleware should do with a `Request` object.
    ///
    /// An error here is propagated by the containing Chain to, first, this Middleware's
    /// `catch` method, then every subsequent `BeforeMiddleware`'s `catch` methods until one returns
    /// Ok(()) or the Chain's `Handler` is reached, at which point the `Handler`'s `catch` method is
    /// called to produce an error Response.
    fn before(&self, &mut Request) -> IronResult<()>;

    /// Try to `catch` an error thrown by this Middleware or a previous `BeforeMiddleware`.
    ///
    /// Should only return Ok(()) if the error has been completely handled and a Chain
    /// can proceed as normal.
    fn catch(&self, _: &mut Request, err: Box<Error>) -> IronResult<()> {
        Err(err)
    }
}

/// `AfterMiddleware` are fired after a `Handler` is called inside of a Chain.
///
/// `AfterMiddleware` receive both a `Request` and a `Response` and are responsible for doing
/// any response-post-processing.
///
/// `AfterMiddleware` should *not* usually overwrite the contents of a Response. In
/// the common case, a complete response is generated by the Chain's `Handler` and
/// `AfterMiddleware` simply do post-processing of that Response, such as
/// adding headers or logging.
pub trait AfterMiddleware: Send + Sync {
    /// Do whatever work this middleware needs to do with both a `Request` and `Response` objects.
    ///
    /// An error here is propagated by the containing Chain down to this and any later
    /// `AfterMiddleware`'s `catch` methods, which can attempt to handle the error or modify
    /// the `Response` to indicate to a client that something went wrong.
    fn after(&self, &mut Request, &mut Response) -> IronResult<()>;

    /// Try to catch an error thrown by previous `AfterMiddleware`, the `Handler`, or a previous
    /// `BeforeMiddleware`.
    ///
    /// The response was generated by the `catch` function of `Handler`s and is abnormal or
    /// errored in some way.
    fn catch(&self, _: &mut Request, _: &mut Response, err: Box<Error>) -> IronResult<()> {
        Err(err)
    }
}

/// AroundMiddleware are used to wrap and replace the `Handler` in a Chain.
///
/// AroundMiddleware must themselves be `Handler`s, and can integrate an existing
/// `Handler` through the around method, which is called once on insertion
/// into a Chain.
pub trait AroundMiddleware: Handler {
    /// Incorporate another `Handler` into this AroundMiddleware.
    ///
    /// Usually this means wrapping the handler and editing the `Request` on the
    /// way in and the `Response` on the way out.
    fn around(&mut self, handler: Box<Handler + Send + Sync>);
}

/// Chain's hold `BeforeMiddleware`, a `Handler`, and `AfterMiddleware` and are responsible
/// for correctly dispatching a `Request` through them.
///
/// Chain's are handlers, and most of their work is done in the call method of their
/// `Handler` implementation.
pub trait Chain: Handler {
    /// Create a new Chain from a `Handler`.
    fn new<H: Handler>(H) -> Self;

    /// Link both a before and after middleware to the chain at once.
    ///
    /// Middleware that have a Before and After piece should have a constructor
    /// which returns both as a tuple, so it can be passed directly to link.
    fn link<B, A>(&mut self, (B, A)) where A: AfterMiddleware, B: BeforeMiddleware;

    /// Link a `BeforeMiddleware` to the Chain.
    fn link_before<B>(&mut self, B) where B: BeforeMiddleware;

    /// Link a `AfterMiddleware` to the Chain.
    fn link_after<A>(&mut self, A) where A: AfterMiddleware;

    /// Wrap the Chain's `Handler` using an AroundMiddleware.
    fn around<A>(&mut self, A) where A: AroundMiddleware;
}

/// The default Chain used in Iron.
///
/// For almost all intents and purposes, this is synonymous with the
/// Chain trait and is the canonical implementation. However, Chain
/// is left as a trait for future interoperability with other
/// frameworks.
pub struct ChainBuilder {
    befores: Vec<Box<BeforeMiddleware + Send + Sync>>,
    afters: Vec<Box<AfterMiddleware + Send + Sync>>,
    handler: Box<Handler + Send + Sync>
}

impl ChainBuilder {
    /// Construct a new ChainBuilder from a `Handler`.
    pub fn new<H: Handler>(handler: H) -> ChainBuilder {
        ChainBuilder {
            befores: vec![],
            afters: vec![],
            handler: box handler as Box<Handler + Send + Sync>
        }
    }
}

impl Chain for ChainBuilder {
    fn new<H: Handler>(handler: H) -> ChainBuilder {
        ChainBuilder {
            befores: vec![],
            afters: vec![],
            handler: box handler as Box<Handler + Send + Sync>
        }
    }

    fn link<B, A>(&mut self, link: (B, A))
    where A: AfterMiddleware, B: BeforeMiddleware {
        let (before, after) = link;
        self.befores.push(box before as Box<BeforeMiddleware + Send + Sync>);
        self.afters.push(box after as Box<AfterMiddleware + Send + Sync>);
    }

    fn link_before<B>(&mut self, before: B) where B: BeforeMiddleware {
        self.befores.push(box before as Box<BeforeMiddleware + Send + Sync>);
    }

    fn link_after<A>(&mut self, after: A) where A: AfterMiddleware {
        self.afters.push(box after as Box<AfterMiddleware + Send + Sync>);
    }

    fn around<A>(&mut self, mut around: A) where A: AroundMiddleware {
        use std::mem;

        let old = mem::replace(&mut self.handler, box Nop as Box<Handler + Send + Sync>);
        around.around(old);
        self.handler = box around as Box<Handler + Send + Sync>;
    }
}

impl Handler for ChainBuilder {
    fn call(&self, req: &mut Request) -> IronResult<Response> {
        let before_result = helpers::run_befores(req, self.befores.as_slice(), None);

        let (res, err) = match before_result {
            Ok(()) => match self.handler.call(req) {
                Ok(res) => (res, None),
                Err(e) => helpers::run_handler_catch(req, e, &self.handler)
            },
            Err(e) => helpers::run_handler_catch(req, e, &self.handler)
        };

        helpers::run_afters(req, res, err, self.afters.as_slice())
    }

    fn catch(&self, req: &mut Request, err: Box<Error>) -> (Response, IronResult<()>) {
        let before_result = helpers::run_befores(req, self.befores.as_slice(), Some(err));

        let (res, err) = match before_result {
            Ok(()) => match self.handler.call(req) {
                Ok(res) => (res, None),
                Err(e) => helpers::run_handler_catch(req, e, &self.handler)
            },
            Err(e) => helpers::run_handler_catch(req, e, &self.handler)
        };

        match helpers::run_afters(req, res, err, self.afters.as_slice()) {
            Ok(res) => (res, Ok(())),
            // FIXME: Make 500
            Err(err) => (Response::new(), Err(err))
        }
    }
}

impl Handler for fn(&mut Request) -> IronResult<Response> {
    fn call(&self, req: &mut Request) -> IronResult<Response> {
        (*self)(req)
    }

    fn catch(&self, _: &mut Request, err: Box<Error>) -> (Response, IronResult<()>) {
        // FIXME: Make Response a 500
        (Response::new(), Err(err))
    }
}

pub struct Nop;

impl Handler for Nop {
    fn call(&self, _: &mut Request) -> IronResult<Response> {
        Ok(Response::new())
    }

    fn catch(&self, _: &mut Request, err: Box<Error>) -> (Response, IronResult<()>) {
        // FIXME: Make Response a 500
        (Response::new(), Err(err))
    }
}

impl Handler for Box<Handler + Send + Sync> {
    fn call(&self, req: &mut Request) -> IronResult<Response> {
        self.call(req)
    }

    fn catch(&self, req: &mut Request, err: Box<Error>) -> (Response, IronResult<()>) {
        self.catch(req, err)
    }
}

impl Handler for Arc<Box<Handler + Send + Sync>> {
    fn call(&self, req: &mut Request) -> IronResult<Response> {
        self.call(req)
    }

    fn catch(&self, req: &mut Request, err: Box<Error>) -> (Response, IronResult<()>) {
        self.catch(req, err)
    }
}

mod helpers {
    use super::super::{Request, Response, IronResult};
    use super::{AfterMiddleware, BeforeMiddleware, Handler};
    use error::Error;

    pub fn run_befores(req: &mut Request, befores: &[Box<BeforeMiddleware>], err: Option<Box<Error>>) -> IronResult<()> {
        match err {
            Some(mut e) => {
                for (i, before) in befores.iter().enumerate() {
                    match before.catch(req, e) {
                        Ok(_) => return run_befores(req, befores, None),
                        Err(new) => e = new
                    }
                }
                Err(e)
            },

            None => {
                for (i, before) in befores.iter().enumerate() {
                    match before.before(req) {
                        Ok(_) => (),
                        Err(err) => return run_befores(req, befores.slice_from(i), Some(err))
                    }
                }
                Ok(())
            }
        }
    }

    pub fn run_afters(req: &mut Request, mut res: Response, err: Option<Box<Error>>,
                  afters: &[Box<AfterMiddleware>]) -> IronResult<Response> {
        match err {
            Some(mut e) => {
                for (i, after) in afters.iter().enumerate() {
                    match after.catch(req, &mut res, e) {
                        Ok(_) => return run_afters(req, res, None, afters),
                        Err(new) => e = new
                    }
                }
                Err(e)
            },

            None => {
                for (i, after) in afters.iter().enumerate() {
                    match after.after(req, &mut res) {
                        Ok(_) => (),
                        Err(err) => return run_afters(req, res, Some(err), afters.slice_from(i))
                    }
                }
                Ok(res)
            }
        }
    }

    pub fn run_handler_catch(req: &mut Request, err: Box<Error>,
                         handler: &Box<Handler>) -> (Response, Option<Box<Error>>) {
        match handler.catch(req, err) {
            (res, Ok(())) => (res, None),
            (res, Err(e)) => (res, Some(e))
        }
    }
}

