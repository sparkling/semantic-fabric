use super::*;
impl<'query> State<'query> {
    pub(super) fn new(node_limit: usize, depth_limit: usize) -> Self {
        Self {
            work: Vec::new(),
            visited: 0,
            node_limit,
            depth_limit,
            variables_lr: HashMap::new(),
            variables_rl: HashMap::new(),
            pattern_blanks_lr: HashMap::new(),
            pattern_blanks_rl: HashMap::new(),
            template_blanks_lr: HashMap::new(),
            template_blanks_rl: HashMap::new(),
        }
    }

    pub(super) fn seed_select_outputs(
        &mut self,
        left: &'query Query,
        right: &'query Query,
    ) -> Result<bool, ()> {
        let (Query::Select { pattern: left, .. }, Query::Select { pattern: right, .. }) =
            (left, right)
        else {
            return Ok(true);
        };
        let (left, right) = match (
            super::expr::select_outputs(left),
            super::expr::select_outputs(right),
        ) {
            (Some(left), Some(right)) => (left, right),
            (None, None) => return Ok(true),
            _ => return Ok(false),
        };
        if left.len() != right.len() {
            return Ok(false);
        }
        for (left, right) in left.iter().zip(right) {
            if left.as_str() != right.as_str() || !self.bind_variable(left, right)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn run(&mut self) -> Result<bool, ()> {
        while let Some(frame) = self.work.pop() {
            self.visited = self.visited.checked_add(1).ok_or(())?;
            if self.visited > self.node_limit {
                return Err(());
            }
            if !self.visit(frame)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub(super) fn push(
        &mut self,
        pair: Pair<'query>,
        depth: usize,
        scope: BlankScope,
    ) -> Result<(), ()> {
        if depth > self.depth_limit || self.work.len().checked_add(1).ok_or(())? > self.node_limit {
            return Err(());
        }
        #[cfg(test)]
        if super::take_forced_work_reservation_failure() {
            return Err(());
        }
        self.work.try_reserve(1).map_err(|_| ())?;
        self.work.push(Frame { pair, depth, scope });
        Ok(())
    }

    pub(super) fn visit(&mut self, frame: Frame<'query>) -> Result<bool, ()> {
        let next = frame.depth.checked_add(1).ok_or(())?;
        match frame.pair {
            Pair::Query(left, right) => self.query(left, right, next),
            Pair::Dataset(left, right) => Ok(left == right),
            Pair::Graph(left, right) => self.graph(left, right, next, frame.scope),
            Pair::Expression(left, right) => self.expression(left, right, next, frame.scope),
            Pair::Path(left, right) => self.path(left, right, next, frame.scope),
            Pair::Aggregate(left, right) => self.aggregate(left, right, next, frame.scope),
            Pair::Order(left, right) => self.order(left, right, next, frame.scope),
            Pair::Triple(left, right) => self.triple(left, right, next, frame.scope),
            Pair::Term(left, right) => self.term(left, right, next, frame.scope),
            Pair::NamedPattern(left, right) => self.named_pattern(left, right, next, frame.scope),
            Pair::GroundTerm(left, right) => self.ground_term(left, right, next, frame.scope),
            Pair::GroundTriple(left, right) => self.ground_triple(left, right, next, frame.scope),
            Pair::Variable(left, right) => self.bind_variable(left, right),
            Pair::Blank(left, right) => self.bind_blank(left, right, frame.scope),
        }
    }

    pub(super) fn query(
        &mut self,
        left: &'query Query,
        right: &'query Query,
        next: usize,
    ) -> Result<bool, ()> {
        match (left, right) {
            (
                Query::Select {
                    dataset: ld,
                    pattern: lp,
                    base_iri: lb,
                },
                Query::Select {
                    dataset: rd,
                    pattern: rp,
                    base_iri: rb,
                },
            )
            | (
                Query::Describe {
                    dataset: ld,
                    pattern: lp,
                    base_iri: lb,
                },
                Query::Describe {
                    dataset: rd,
                    pattern: rp,
                    base_iri: rb,
                },
            )
            | (
                Query::Ask {
                    dataset: ld,
                    pattern: lp,
                    base_iri: lb,
                },
                Query::Ask {
                    dataset: rd,
                    pattern: rp,
                    base_iri: rb,
                },
            ) => {
                if lb != rb || !self.option_dataset(ld, rd, next)? {
                    return Ok(false);
                }
                self.push(Pair::Graph(lp, rp), next, BlankScope::Pattern)?;
                Ok(true)
            }
            (
                Query::Construct {
                    template: lt,
                    dataset: ld,
                    pattern: lp,
                    base_iri: lb,
                },
                Query::Construct {
                    template: rt,
                    dataset: rd,
                    pattern: rp,
                    base_iri: rb,
                },
            ) => {
                if lb != rb || lt.len() != rt.len() || !self.option_dataset(ld, rd, next)? {
                    return Ok(false);
                }
                self.push(Pair::Graph(lp, rp), next, BlankScope::Pattern)?;
                for (left, right) in lt.iter().zip(rt) {
                    self.push(Pair::Triple(left, right), next, BlankScope::Template)?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn graph(
        &mut self,
        left: &'query GraphPattern,
        right: &'query GraphPattern,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        match (left, right) {
            (GraphPattern::Bgp { patterns: left }, GraphPattern::Bgp { patterns: right }) => {
                self.triples(left, right, next, scope)
            }
            (
                GraphPattern::Path {
                    subject: ls,
                    path: lp,
                    object: lo,
                },
                GraphPattern::Path {
                    subject: rs,
                    path: rp,
                    object: ro,
                },
            ) => {
                self.push(Pair::Term(ls, rs), next, scope)?;
                self.push(Pair::Path(lp, rp), next, scope)?;
                self.push(Pair::Term(lo, ro), next, scope)?;
                Ok(true)
            }
            (
                GraphPattern::Join {
                    left: ll,
                    right: lr,
                },
                GraphPattern::Join {
                    left: rl,
                    right: rr,
                },
            )
            | (
                GraphPattern::Lateral {
                    left: ll,
                    right: lr,
                },
                GraphPattern::Lateral {
                    left: rl,
                    right: rr,
                },
            )
            | (
                GraphPattern::Union {
                    left: ll,
                    right: lr,
                },
                GraphPattern::Union {
                    left: rl,
                    right: rr,
                },
            )
            | (
                GraphPattern::Minus {
                    left: ll,
                    right: lr,
                },
                GraphPattern::Minus {
                    left: rl,
                    right: rr,
                },
            ) => {
                self.push(Pair::Graph(ll, rl), next, scope)?;
                self.push(Pair::Graph(lr, rr), next, scope)?;
                Ok(true)
            }
            (
                GraphPattern::LeftJoin {
                    left: ll,
                    right: lr,
                    expression: le,
                },
                GraphPattern::LeftJoin {
                    left: rl,
                    right: rr,
                    expression: re,
                },
            ) => {
                if !self.option_expression(le, re, next, scope)? {
                    return Ok(false);
                }
                self.push(Pair::Graph(ll, rl), next, scope)?;
                self.push(Pair::Graph(lr, rr), next, scope)?;
                Ok(true)
            }
            (
                GraphPattern::Filter {
                    expr: le,
                    inner: li,
                },
                GraphPattern::Filter {
                    expr: re,
                    inner: ri,
                },
            ) => {
                self.push(Pair::Expression(le, re), next, scope)?;
                self.push(Pair::Graph(li, ri), next, scope)?;
                Ok(true)
            }
            (
                GraphPattern::Graph {
                    name: ln,
                    inner: li,
                },
                GraphPattern::Graph {
                    name: rn,
                    inner: ri,
                },
            ) => {
                self.push(Pair::NamedPattern(ln, rn), next, scope)?;
                self.push(Pair::Graph(li, ri), next, scope)?;
                Ok(true)
            }
            (
                GraphPattern::Extend {
                    inner: li,
                    variable: lv,
                    expression: le,
                },
                GraphPattern::Extend {
                    inner: ri,
                    variable: rv,
                    expression: re,
                },
            ) => {
                self.push(Pair::Graph(li, ri), next, scope)?;
                self.push(Pair::Variable(lv, rv), next, scope)?;
                self.push(Pair::Expression(le, re), next, scope)?;
                Ok(true)
            }
            (
                GraphPattern::Values {
                    variables: lv,
                    bindings: lb,
                },
                GraphPattern::Values {
                    variables: rv,
                    bindings: rb,
                },
            ) => self.values(lv, lb, rv, rb, next, scope),
            (
                GraphPattern::OrderBy {
                    inner: li,
                    expression: le,
                },
                GraphPattern::OrderBy {
                    inner: ri,
                    expression: re,
                },
            ) => {
                if le.len() != re.len() {
                    return Ok(false);
                }
                self.push(Pair::Graph(li, ri), next, scope)?;
                for (left, right) in le.iter().zip(re) {
                    self.push(Pair::Order(left, right), next, scope)?;
                }
                Ok(true)
            }
            (
                GraphPattern::Project {
                    inner: li,
                    variables: lv,
                },
                GraphPattern::Project {
                    inner: ri,
                    variables: rv,
                },
            ) => {
                if lv.len() != rv.len() {
                    return Ok(false);
                }
                self.push(Pair::Graph(li, ri), next, scope)?;
                for (left, right) in lv.iter().zip(rv) {
                    self.push(Pair::Variable(left, right), next, scope)?;
                }
                Ok(true)
            }
            (GraphPattern::Distinct { inner: left }, GraphPattern::Distinct { inner: right })
            | (GraphPattern::Reduced { inner: left }, GraphPattern::Reduced { inner: right }) => {
                self.push(Pair::Graph(left, right), next, scope)?;
                Ok(true)
            }
            (
                GraphPattern::Slice {
                    inner: li,
                    start: ls,
                    length: ll,
                },
                GraphPattern::Slice {
                    inner: ri,
                    start: rs,
                    length: rl,
                },
            ) => {
                if ls != rs || ll != rl {
                    return Ok(false);
                }
                self.push(Pair::Graph(li, ri), next, scope)?;
                Ok(true)
            }
            (
                GraphPattern::Group {
                    inner: li,
                    variables: lv,
                    aggregates: la,
                },
                GraphPattern::Group {
                    inner: ri,
                    variables: rv,
                    aggregates: ra,
                },
            ) => {
                if lv.len() != rv.len() || la.len() != ra.len() {
                    return Ok(false);
                }
                self.push(Pair::Graph(li, ri), next, scope)?;
                for (left, right) in lv.iter().zip(rv) {
                    self.push(Pair::Variable(left, right), next, scope)?;
                }
                for ((lv, la), (rv, ra)) in la.iter().zip(ra) {
                    self.push(Pair::Variable(lv, rv), next, scope)?;
                    self.push(Pair::Aggregate(la, ra), next, scope)?;
                }
                Ok(true)
            }
            (
                GraphPattern::Service {
                    name: ln,
                    inner: li,
                    silent: ls,
                },
                GraphPattern::Service {
                    name: rn,
                    inner: ri,
                    silent: rs,
                },
            ) => {
                if ls != rs {
                    return Ok(false);
                }
                self.push(Pair::NamedPattern(ln, rn), next, scope)?;
                self.push(Pair::Graph(li, ri), next, scope)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn triples(
        &mut self,
        left: &'query [TriplePattern],
        right: &'query [TriplePattern],
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        if left.len() != right.len() {
            return Ok(false);
        }
        for (left, right) in left.iter().zip(right) {
            self.push(Pair::Triple(left, right), next, scope)?;
        }
        Ok(true)
    }
    pub(super) fn option_expression(
        &mut self,
        left: &'query Option<Expression>,
        right: &'query Option<Expression>,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        match (left, right) {
            (None, None) => Ok(true),
            (Some(left), Some(right)) => {
                self.push(Pair::Expression(left, right), next, scope)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn values(
        &mut self,
        lv: &'query [Variable],
        lb: &'query [Vec<Option<GroundTerm>>],
        rv: &'query [Variable],
        rb: &'query [Vec<Option<GroundTerm>>],
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        if lv.len() != rv.len() || lb.len() != rb.len() {
            return Ok(false);
        }
        for (left, right) in lv.iter().zip(rv) {
            self.push(Pair::Variable(left, right), next, scope)?;
        }
        for (left, right) in lb.iter().zip(rb) {
            if left.len() != right.len() {
                return Ok(false);
            }
            for (left, right) in left.iter().zip(right) {
                match (left, right) {
                    (None, None) => {}
                    (Some(left), Some(right)) => {
                        self.push(Pair::GroundTerm(left, right), next, scope)?
                    }
                    _ => return Ok(false),
                }
            }
        }
        Ok(true)
    }
}
