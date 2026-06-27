//! Three-valued (Kleene K3) logic.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tri {
    True,
    False,
    Unknown,
}

#[inline]
pub fn b(x: bool) -> Tri {
    if x {
        Tri::True
    } else {
        Tri::False
    }
}

#[inline]
pub fn not3(t: Tri) -> Tri {
    match t {
        Tri::True => Tri::False,
        Tri::False => Tri::True,
        Tri::Unknown => Tri::Unknown,
    }
}

/// AND: False dominates; else Unknown if any Unknown; else True. Short-circuits on False.
pub fn and3<I: IntoIterator<Item = Tri>>(it: I) -> Tri {
    let mut u = false;
    for t in it {
        match t {
            Tri::False => return Tri::False,
            Tri::Unknown => u = true,
            Tri::True => {}
        }
    }
    if u {
        Tri::Unknown
    } else {
        Tri::True
    }
}

/// OR: True dominates; else Unknown if any Unknown; else False. Short-circuits on True.
pub fn or3<I: IntoIterator<Item = Tri>>(it: I) -> Tri {
    let mut u = false;
    for t in it {
        match t {
            Tri::True => return Tri::True,
            Tri::Unknown => u = true,
            Tri::False => {}
        }
    }
    if u {
        Tri::Unknown
    } else {
        Tri::False
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_truth_table() {
        assert_eq!(not3(Tri::True), Tri::False);
        assert_eq!(not3(Tri::False), Tri::True);
        assert_eq!(not3(Tri::Unknown), Tri::Unknown);
    }

    #[test]
    fn and_truth_table() {
        use Tri::*;
        assert_eq!(and3([True, True]), True);
        assert_eq!(and3([True, False]), False);
        assert_eq!(and3([Unknown, False]), False);
        assert_eq!(and3([True, Unknown]), Unknown);
        assert_eq!(and3([Unknown, Unknown]), Unknown);
        assert_eq!(and3(std::iter::empty()), True);
    }

    #[test]
    fn or_truth_table() {
        use Tri::*;
        assert_eq!(or3([False, False]), False);
        assert_eq!(or3([True, False]), True);
        assert_eq!(or3([Unknown, True]), True);
        assert_eq!(or3([False, Unknown]), Unknown);
        assert_eq!(or3([Unknown, Unknown]), Unknown);
        assert_eq!(or3(std::iter::empty()), False);
    }
}
