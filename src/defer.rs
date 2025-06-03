pub struct Defer<F: FnMut()>(F);

pub fn defer<F>(f: F) -> Defer<F>
where
    F: FnMut(),
{
    Defer(f)
}

impl<F> Drop for Defer<F>
where
    F: FnMut(),
{
    fn drop(&mut self) {
        (self.0)();
    }
}
