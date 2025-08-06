pub fn second<A, B>((_, b): (A, B)) -> B {
    b
}

pub fn first<A, B>((a, _): (A, B)) -> A {
    a
}
