use super::*;

impl<'query> State<'query> {
    pub(super) fn option_dataset(
        &mut self,
        left: &'query Option<QueryDataset>,
        right: &'query Option<QueryDataset>,
        next: usize,
    ) -> Result<bool, ()> {
        match (left, right) {
            (None, None) => Ok(true),
            (Some(left), Some(right)) => {
                self.push(Pair::Dataset(left, right), next, BlankScope::Pattern)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn expression(
        &mut self,
        left: &'query Expression,
        right: &'query Expression,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        use Expression as E;
        match (left, right) {
            (E::NamedNode(left), E::NamedNode(right)) => Ok(left == right),
            (E::Literal(left), E::Literal(right)) => Ok(left == right),
            (E::Variable(left), E::Variable(right)) | (E::Bound(left), E::Bound(right)) => {
                self.bind_variable(left, right)
            }
            (E::Or(ll, lr), E::Or(rl, rr))
            | (E::And(ll, lr), E::And(rl, rr))
            | (E::Equal(ll, lr), E::Equal(rl, rr))
            | (E::SameTerm(ll, lr), E::SameTerm(rl, rr))
            | (E::Greater(ll, lr), E::Greater(rl, rr))
            | (E::GreaterOrEqual(ll, lr), E::GreaterOrEqual(rl, rr))
            | (E::Less(ll, lr), E::Less(rl, rr))
            | (E::LessOrEqual(ll, lr), E::LessOrEqual(rl, rr))
            | (E::Add(ll, lr), E::Add(rl, rr))
            | (E::Subtract(ll, lr), E::Subtract(rl, rr))
            | (E::Multiply(ll, lr), E::Multiply(rl, rr))
            | (E::Divide(ll, lr), E::Divide(rl, rr)) => {
                self.push(Pair::Expression(ll, rl), next, scope)?;
                self.push(Pair::Expression(lr, rr), next, scope)?;
                Ok(true)
            }
            (E::In(left, ls), E::In(right, rs)) => {
                if ls.len() != rs.len() {
                    return Ok(false);
                }
                self.push(Pair::Expression(left, right), next, scope)?;
                for (left, right) in ls.iter().zip(rs) {
                    self.push(Pair::Expression(left, right), next, scope)?;
                }
                Ok(true)
            }
            (E::Coalesce(ls), E::Coalesce(rs)) => {
                if ls.len() != rs.len() {
                    return Ok(false);
                }
                for (left, right) in ls.iter().zip(rs) {
                    self.push(Pair::Expression(left, right), next, scope)?;
                }
                Ok(true)
            }
            (E::UnaryPlus(left), E::UnaryPlus(right))
            | (E::UnaryMinus(left), E::UnaryMinus(right))
            | (E::Not(left), E::Not(right)) => {
                self.push(Pair::Expression(left, right), next, scope)?;
                Ok(true)
            }
            (E::Exists(left), E::Exists(right)) => {
                self.push(Pair::Graph(left, right), next, BlankScope::Pattern)?;
                Ok(true)
            }
            (E::If(la, lb, lc), E::If(ra, rb, rc)) => {
                self.push(Pair::Expression(la, ra), next, scope)?;
                self.push(Pair::Expression(lb, rb), next, scope)?;
                self.push(Pair::Expression(lc, rc), next, scope)?;
                Ok(true)
            }
            (E::FunctionCall(lf, la), E::FunctionCall(rf, ra)) => {
                if lf != rf || la.len() != ra.len() {
                    return Ok(false);
                }
                for (left, right) in la.iter().zip(ra) {
                    self.push(Pair::Expression(left, right), next, scope)?;
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(super) fn path(
        &mut self,
        left: &'query PropertyPathExpression,
        right: &'query PropertyPathExpression,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        use PropertyPathExpression as P;
        match (left, right) {
            (P::NamedNode(left), P::NamedNode(right)) => Ok(left == right),
            (P::Reverse(left), P::Reverse(right))
            | (P::ZeroOrMore(left), P::ZeroOrMore(right))
            | (P::OneOrMore(left), P::OneOrMore(right))
            | (P::ZeroOrOne(left), P::ZeroOrOne(right)) => {
                self.push(Pair::Path(left, right), next, scope)?;
                Ok(true)
            }
            (P::Sequence(ll, lr), P::Sequence(rl, rr))
            | (P::Alternative(ll, lr), P::Alternative(rl, rr)) => {
                self.push(Pair::Path(ll, rl), next, scope)?;
                self.push(Pair::Path(lr, rr), next, scope)?;
                Ok(true)
            }
            (P::NegatedPropertySet(left), P::NegatedPropertySet(right)) => Ok(left == right),
            _ => Ok(false),
        }
    }
    pub(super) fn aggregate(
        &mut self,
        left: &'query AggregateExpression,
        right: &'query AggregateExpression,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        match (left, right) {
            (
                AggregateExpression::CountSolutions { distinct: left },
                AggregateExpression::CountSolutions { distinct: right },
            ) => Ok(left == right),
            (
                AggregateExpression::FunctionCall {
                    name: ln,
                    expr: le,
                    distinct: ld,
                },
                AggregateExpression::FunctionCall {
                    name: rn,
                    expr: re,
                    distinct: rd,
                },
            ) => {
                if ln != rn || ld != rd {
                    return Ok(false);
                }
                self.push(Pair::Expression(le, re), next, scope)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn order(
        &mut self,
        left: &'query OrderExpression,
        right: &'query OrderExpression,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        match (left, right) {
            (OrderExpression::Asc(left), OrderExpression::Asc(right))
            | (OrderExpression::Desc(left), OrderExpression::Desc(right)) => {
                self.push(Pair::Expression(left, right), next, scope)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn triple(
        &mut self,
        left: &'query TriplePattern,
        right: &'query TriplePattern,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        self.push(Pair::Term(&left.subject, &right.subject), next, scope)?;
        self.push(
            Pair::NamedPattern(&left.predicate, &right.predicate),
            next,
            scope,
        )?;
        self.push(Pair::Term(&left.object, &right.object), next, scope)?;
        Ok(true)
    }
    pub(super) fn term(
        &mut self,
        left: &'query TermPattern,
        right: &'query TermPattern,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        match (left, right) {
            (TermPattern::NamedNode(left), TermPattern::NamedNode(right)) => Ok(left == right),
            (TermPattern::BlankNode(left), TermPattern::BlankNode(right)) => self
                .push(Pair::Blank(left.as_str(), right.as_str()), next, scope)
                .map(|_| true),
            (TermPattern::Literal(left), TermPattern::Literal(right)) => Ok(left == right),
            (TermPattern::Triple(left), TermPattern::Triple(right)) => self
                .push(Pair::Triple(left, right), next, scope)
                .map(|_| true),
            (TermPattern::Variable(left), TermPattern::Variable(right)) => {
                self.bind_variable(left, right)
            }
            _ => Ok(false),
        }
    }
    pub(super) fn named_pattern(
        &mut self,
        left: &'query NamedNodePattern,
        right: &'query NamedNodePattern,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        match (left, right) {
            (NamedNodePattern::NamedNode(left), NamedNodePattern::NamedNode(right)) => {
                Ok(left == right)
            }
            (NamedNodePattern::Variable(left), NamedNodePattern::Variable(right)) => self
                .push(Pair::Variable(left, right), next, scope)
                .map(|_| true),
            _ => Ok(false),
        }
    }
    pub(super) fn ground_term(
        &mut self,
        left: &'query GroundTerm,
        right: &'query GroundTerm,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        match (left, right) {
            (GroundTerm::NamedNode(left), GroundTerm::NamedNode(right)) => Ok(left == right),
            (GroundTerm::Literal(left), GroundTerm::Literal(right)) => Ok(left == right),
            (GroundTerm::Triple(left), GroundTerm::Triple(right)) => self
                .push(Pair::GroundTriple(left, right), next, scope)
                .map(|_| true),
            _ => Ok(false),
        }
    }
    pub(super) fn ground_triple(
        &mut self,
        left: &'query GroundTriple,
        right: &'query GroundTriple,
        next: usize,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        if left.subject != right.subject || left.predicate != right.predicate {
            return Ok(false);
        }
        self.push(Pair::GroundTerm(&left.object, &right.object), next, scope)?;
        Ok(true)
    }

    pub(super) fn bind_variable(
        &mut self,
        left: &'query Variable,
        right: &'query Variable,
    ) -> Result<bool, ()> {
        bind(
            &mut self.variables_lr,
            &mut self.variables_rl,
            left.as_str(),
            right.as_str(),
            self.node_limit,
        )
    }
    pub(super) fn bind_blank(
        &mut self,
        left: &'query str,
        right: &'query str,
        scope: BlankScope,
    ) -> Result<bool, ()> {
        match scope {
            BlankScope::Pattern => bind(
                &mut self.pattern_blanks_lr,
                &mut self.pattern_blanks_rl,
                left,
                right,
                self.node_limit,
            ),
            BlankScope::Template => bind(
                &mut self.template_blanks_lr,
                &mut self.template_blanks_rl,
                left,
                right,
                self.node_limit,
            ),
        }
    }
}

fn bind<'query>(
    forward: &mut HashMap<&'query str, &'query str>,
    reverse: &mut HashMap<&'query str, &'query str>,
    left: &'query str,
    right: &'query str,
    limit: usize,
) -> Result<bool, ()> {
    if let Some(mapped) = forward.get(left) {
        return Ok(*mapped == right);
    }
    if reverse.contains_key(right) || forward.len() >= limit || reverse.len() >= limit {
        return Ok(false);
    }
    forward.try_reserve(1).map_err(|_| ())?;
    reverse.try_reserve(1).map_err(|_| ())?;
    forward.insert(left, right);
    reverse.insert(right, left);
    Ok(true)
}

pub(super) fn select_outputs(pattern: &GraphPattern) -> Option<&[Variable]> {
    let mut pattern = pattern;
    loop {
        match pattern {
            GraphPattern::Slice { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner } => pattern = inner,
            GraphPattern::Project { variables, .. } => return Some(variables),
            _ => return None,
        }
    }
}
