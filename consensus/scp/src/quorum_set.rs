// Copyright (c) 2018-2020 MobileCoin Inc.

//! The quorum set is the essential unit of trust in SCP.
//!
//! A quorum set includes the members of the network, which a given node trusts and depends on.
use mc_common::{HashMap, HashSet, NodeID, ResponderId};
use mc_crypto_digestible::Digestible;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
    hash::Hash,
    iter::FromIterator,
};

use crate::{
    core_types::{GenericNodeId, Value},
    msg::Msg,
    predicates::Predicate,
};

/// The quorum set defining the trusted set of peers.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash, Digestible)]
pub struct QuorumSet<ID: GenericNodeId = NodeID> {
    /// Threshold (how many members do we need to reach quorum).
    pub threshold: u32,

    /// Members.
    pub members: Vec<QuorumSetMember<ID>>,
}

/// A member in a QuorumSet. Can be either a Node or another QuorumSet.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq, Hash, Digestible)]
#[serde(tag = "type", content = "args")]
pub enum QuorumSetMember<ID: GenericNodeId> {
    /// A single trusted entity with an identity.
    Node(ID),

    /// A quorum set can also be a member of a quorum set.
    InnerSet(QuorumSet<ID>),
}

impl<
        ID: GenericNodeId
            + Clone
            + Debug
            + Display
            + Serialize
            + DeserializeOwned
            + Eq
            + PartialEq
            + Hash,
    > QuorumSet<ID>
{
    /// Create a new quorum set.
    pub fn new(threshold: u32, members: Vec<QuorumSetMember<ID>>) -> Self {
        Self { threshold, members }
    }

    /// Create a new quorum set from the given node IDs.
    pub fn new_with_node_ids(threshold: u32, node_ids: Vec<ID>) -> Self {
        Self::new(
            threshold,
            node_ids.into_iter().map(QuorumSetMember::Node).collect(),
        )
    }

    /// Create a new quorum set from the given inner sets.
    pub fn new_with_inner_sets(threshold: u32, inner_sets: Vec<Self>) -> Self {
        Self::new(
            threshold,
            inner_sets
                .into_iter()
                .map(QuorumSetMember::InnerSet)
                .collect(),
        )
    }

    /// A quorum set with no members and a threshold of 0.
    pub fn empty() -> Self {
        Self::new(0, vec![])
    }

    /// Returns a flattened set of all nodes contained in q and its nested QSets.
    pub fn nodes(&self) -> HashSet<ID> {
        let mut result = HashSet::<ID>::default();
        for member in self.members.iter() {
            match member {
                QuorumSetMember::Node(node_id) => {
                    result.insert(node_id.clone());
                }
                QuorumSetMember::InnerSet(qs) => {
                    result.extend(qs.nodes());
                }
            }
        }
        result
    }

    /// Gives the fraction of quorum slices containing the given node.
    /// It assumes that id appears in at most one QuorumSet
    /// (either the top level one or a single reachable nested one)
    /// and then only once in that QuorumSet.
    ///
    /// # Returns
    /// * (numerator, denominator) representing the node's weight.
    pub fn weight(&self, node_id: &ID) -> (u32, u32) {
        for m in self.members.iter() {
            match m {
                QuorumSetMember::Node(N) => {
                    if N == node_id {
                        return (self.threshold, self.members.len() as u32);
                    }
                }
                QuorumSetMember::InnerSet(Q) => {
                    let (num2, denom2) = Q.weight(node_id);
                    if num2 > 0 {
                        return (self.threshold * num2, self.members.len() as u32 * denom2);
                    }
                }
            }
        }

        (0, 1)
    }

    /// Attempts to find a blocking set matching a given predicate `predicate`.
    ///
    /// # Arguments
    /// * `msgs` - A map of ID -> Msg holding the newest message received from each node.
    /// * `pred` - Predicate to apply to the messages.
    ///
    /// # Returns
    /// * (Set of nodes forming a bocking set and matching the predicate, the predicate).
    ///   The set of nodes would be empty if no blocking set matching the predicate was found.
    pub fn findBlockingSet<V: Value, P: Predicate<V, ID>>(
        &self,
        msgs: &HashMap<ID, Msg<V, ID>>,
        pred: P,
    ) -> (HashSet<ID>, P) {
        Self::findBlockingSetHelper(
            self.members.len() as u32 - self.threshold + 1,
            &self.members,
            msgs,
            pred,
            HashSet::default(),
        )
    }

    /// Internal helper method, implementing the logic for finding a blocking set.
    ///
    /// # Arguments
    /// * `needed` - How many more nodes do we need to reach a blocking set.
    /// * `members` - Array of quorum set members we are considering as potential blocking set
    ///    members.
    /// * `msgs` - A map of ID -> Msg holding the newest message received from each node.
    /// * `pred` - Predicate to apply to the messages.
    /// * `node_so_far` - Nodes we have collected so far in our quest for finding a blocking set.
    fn findBlockingSetHelper<V: Value, P: Predicate<V, ID>>(
        needed: u32,
        members: &[QuorumSetMember<ID>],
        msgs: &HashMap<ID, Msg<V, ID>>,
        pred: P,
        nodes_so_far: HashSet<ID>,
    ) -> (HashSet<ID>, P) {
        // If we don't need any more nodes, we're done.
        if needed == 0 {
            return (nodes_so_far, pred);
        }

        // If we need more nodes/sets than we have, we will never find a match.
        if needed as usize > members.len() {
            return (HashSet::default(), pred);
        }

        // See if the first member of our potential nodes/sets allows us to reach a blocking
        // threshold.
        match &members[0] {
            QuorumSetMember::Node(N) => {
                // If we have received a message from this member
                if let Some(msg) = msgs.get(N) {
                    // and the predicate accepts it
                    if let Some(nextPred) = pred.test(msg) {
                        // then add this node to the list of potential matches, and continue
                        // searching.
                        let mut nodes_so_far2 = nodes_so_far;
                        nodes_so_far2.insert(N.clone());
                        return Self::findBlockingSetHelper(
                            needed - 1,
                            &members[1..],
                            msgs,
                            nextPred,
                            nodes_so_far2,
                        );
                    }
                }
            }
            QuorumSetMember::InnerSet(Q) => {
                let (nodes_so_far2, pred2) = Self::findBlockingSetHelper(
                    // "A message reaches blocking threshold at "v" when the number of
                    //  "validators" making the statement plus (recursively) the number
                    // "innerSets" reaching blocking threshold exceeds "n-k"."a
                    // p.9 of the [IETF draft](https://tools.ietf.org/pdf/draft-mazieres-dinrg-scp-04.pdf).
                    Q.members.len() as u32 - Q.threshold + 1,
                    &Q.members,
                    msgs,
                    pred.clone(),
                    nodes_so_far.clone(),
                );
                if !nodes_so_far2.is_empty() {
                    return Self::findBlockingSetHelper(
                        needed - 1,
                        &members[1..],
                        msgs,
                        pred2,
                        nodes_so_far2,
                    );
                }
            }
        }

        // First member didn't get us to a blocking set, move to the next member and try again.
        Self::findBlockingSetHelper(needed, &members[1..], msgs, pred, nodes_so_far)
    }

    /// Attempts to find a quorum matching a given predicate `predicate`.
    ///
    /// # Arguments
    /// * `node_id` - The local node ID.
    /// * `msgs` - A map of ID -> Msg holding the newest message received from each node.
    /// * `pred` - Predicate to apply to the messages.
    ///
    /// # Returns
    /// * (Set of nodes forming a quorum and matching the predicate, the predicate).
    ///   The set of nodes would be empty if no quorum matching the predicate was found.
    pub fn findQuorum<V: Value, P: Predicate<V, ID>>(
        &self,
        node_id: &ID,
        msgs: &HashMap<ID, Msg<V, ID>>,
        pred: P,
    ) -> (HashSet<ID>, P) {
        Self::findQuorumHelper(
            self.threshold,
            &self.members,
            msgs,
            pred,
            HashSet::from_iter(vec![node_id.clone()]),
        )
    }

    /// Internal helper method, implementing the logic for finding a quorum.
    ///
    /// # Arguments
    /// * `threshold` - How many more nodes do we need to reach a quorum.
    /// * `members` - Array of quorum set members we are considering as potential quorum members.
    /// * `msgs` - A map of ID -> Msg holding the newest message received from each node.
    /// * `pred` - Predicate to apply to the messages.
    /// * `node_so_far` - Nodes we have collected so far in our quest for finding a quorum.
    fn findQuorumHelper<V: Value, P: Predicate<V, ID>>(
        threshold: u32,
        members: &[QuorumSetMember<ID>],
        msgs: &HashMap<ID, Msg<V, ID>>,
        pred: P,
        nodes_so_far: HashSet<ID>,
    ) -> (HashSet<ID>, P) {
        // If we don't need any more nodes, we're done.
        if threshold == 0 {
            return (nodes_so_far, pred);
        }

        // If we need more nodes/sets than we have, we will never find a match.
        if threshold as usize > members.len() {
            return (HashSet::default(), pred);
        }

        // See if the first member of our potential nodes/sets allows us to reach quorum.
        match &members[0] {
            QuorumSetMember::Node(N) => {
                // If we already seen this node and it got added to the list of potential
                // quorum-forming nodes, we need one less node to reach quorum.
                if nodes_so_far.contains(N) {
                    return Self::findQuorumHelper(
                        threshold - 1,
                        &members[1..],
                        msgs,
                        pred,
                        nodes_so_far,
                    );
                }

                // If we have received a message from node N
                if let Some(msg) = msgs.get(N) {
                    // and if the predicate accepts it
                    if let Some(nextPred) = pred.test(msg) {
                        // then add this node into the list of potentoal quorum-forming nodes, and
                        // see if we can find a quorum that satisfies it's validators.
                        let mut nodes_so_far_with_N = nodes_so_far.clone();
                        nodes_so_far_with_N.insert(N.clone());

                        let (nodes_so_far2, pred2) = Self::findQuorumHelper(
                            msg.quorum_set.threshold,
                            &msg.quorum_set.members,
                            msgs,
                            nextPred,
                            nodes_so_far_with_N,
                        );
                        if !nodes_so_far2.is_empty() {
                            // We can find a quorum for the node's validators, so consider it a
                            // good potentail fit and keep searching for `threshold - 1` nodes.
                            return Self::findQuorumHelper(
                                threshold - 1,
                                &members[1..],
                                msgs,
                                pred2,
                                nodes_so_far2,
                            );
                        }
                    }
                }
            }
            QuorumSetMember::InnerSet(Q) => {
                // See if we can find quorum for the inner set.
                let (nodes_so_far2, pred2) = Self::findQuorumHelper(
                    Q.threshold,
                    &Q.members,
                    msgs,
                    pred.clone(),
                    nodes_so_far.clone(),
                );
                if !nodes_so_far2.is_empty() {
                    // We found a quorum for the inner set, we need 1 validator less.
                    return Self::findQuorumHelper(
                        threshold - 1,
                        &members[1..],
                        msgs,
                        pred2,
                        nodes_so_far2,
                    );
                }
            }
        }

        // First member didn't get us to a quorum, move to the next member and try again.
        Self::findQuorumHelper(threshold, &members[1..], msgs, pred, nodes_so_far)
    }
}

impl<ID: GenericNodeId + AsRef<ResponderId>> From<&QuorumSet<ID>> for QuorumSet<ResponderId> {
    fn from(src: &QuorumSet<ID>) -> QuorumSet<ResponderId> {
        let members = src
            .members
            .iter()
            .map(|member| match member {
                QuorumSetMember::Node(node_id) => QuorumSetMember::Node(node_id.as_ref().clone()),
                QuorumSetMember::InnerSet(quorum_set) => {
                    QuorumSetMember::InnerSet(quorum_set.into())
                }
            })
            .collect();
        QuorumSet {
            threshold: src.threshold,
            members,
        }
    }
}

#[cfg(test)]
mod quorum_set_tests {
    use super::*;
    use crate::{core_types::*, msg::*, predicates::*, test_utils::test_node_id};
    use mc_common::ResponderId;

    #[test]
    // findBlockingSet returns an empty set when there is no blocking set
    fn test_no_blocking_set() {
        // Node 2 and 3 form a blocking set
        let local_node_quorum_set: QuorumSet = {
            let inner_quorum_set_one = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(2), test_node_id(3), test_node_id(4)],
            );
            let inner_quorum_set_two = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(5), test_node_id(6), test_node_id(7)],
            );
            QuorumSet::new_with_inner_sets(2, vec![inner_quorum_set_one, inner_quorum_set_two])
        };

        let node_2_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(3), test_node_id(4)]);
        let node_5_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(6), test_node_id(7)]);

        let topic = Topic::Prepare(PreparePayload::<u32> {
            B: Ballot::new(1, &[1234, 5678]),
            P: None,
            PP: None,
            CN: 0,
            HN: 0,
        });

        let mut msgs = HashMap::<NodeID, Msg<u32>>::default();
        msgs.insert(
            test_node_id(2),
            Msg::new(test_node_id(2), node_2_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(5),
            Msg::new(test_node_id(5), node_5_quorum_set, 1, topic),
        );
        let (node_ids, _) = local_node_quorum_set.findBlockingSet(
            &msgs,
            FuncPredicate {
                test_fn: &|_msg| true,
            },
        );
        assert_eq!(node_ids.len(), 0);
    }

    #[test]
    // findBlockingSet returns the correct set of nodes when there is a blocking set
    fn test_has_blocking_set() {
        // Node 2 and 3 form a blocking set
        let local_node_quorum_set: QuorumSet = {
            let inner_quorum_set_one = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(2), test_node_id(3), test_node_id(4)],
            );
            let inner_quorum_set_two = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(5), test_node_id(6), test_node_id(7)],
            );
            QuorumSet::new_with_inner_sets(2, vec![inner_quorum_set_one, inner_quorum_set_two])
        };

        let node_2_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(3), test_node_id(4)]);
        let node_3_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(2), test_node_id(4)]);

        let topic = Topic::Prepare(PreparePayload::<u32> {
            B: Ballot::new(1, &[1234, 5678]),
            P: None,
            PP: None,
            CN: 0,
            HN: 0,
        });

        let mut msgs = HashMap::<NodeID, Msg<u32>>::default();
        msgs.insert(
            test_node_id(2),
            Msg::new(test_node_id(2), node_2_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(3),
            Msg::new(test_node_id(3), node_3_quorum_set, 1, topic),
        );

        let (node_ids, _) = local_node_quorum_set.findBlockingSet(
            &msgs,
            FuncPredicate {
                test_fn: &|_msg| true,
            },
        );
        assert_eq!(
            node_ids,
            HashSet::from_iter(vec![test_node_id(2), test_node_id(3)])
        );
    }

    #[test]
    // findBlockingSet returns an empty set if the predicate returns false for the blocking set
    fn test_blocking_set_with_false_predicate() {
        // Node 2 and 3 form a blocking set
        let local_node_quorum_set: QuorumSet = {
            let inner_quorum_set_one = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(2), test_node_id(3), test_node_id(4)],
            );
            let inner_quorum_set_two = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(5), test_node_id(6), test_node_id(7)],
            );
            QuorumSet::new_with_inner_sets(2, vec![inner_quorum_set_one, inner_quorum_set_two])
        };

        let node_2_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(3), test_node_id(4)]);
        let node_3_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(2), test_node_id(4)]);

        let topic = Topic::Prepare(PreparePayload::<u32> {
            B: Ballot::new(1, &[1234, 5678]),
            P: None,
            PP: None,
            CN: 0,
            HN: 0,
        });

        let mut msgs = HashMap::<NodeID, Msg<u32>>::default();
        msgs.insert(
            test_node_id(2),
            Msg::new(test_node_id(2), node_2_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(3),
            Msg::new(test_node_id(3), node_3_quorum_set, 1, topic),
        );

        let (node_ids, _) = local_node_quorum_set.findBlockingSet(
            &msgs,
            FuncPredicate {
                test_fn: &|msg| msg.sender_id == test_node_id(2),
            },
        );
        assert_eq!(node_ids.len(), 0);
    }

    #[test]
    // findQuorum returns an empty set when there is no quorum
    fn test_no_quorum() {
        // Node 2 and 3 form a blocking set. Node 2, 3, 5, 6 form a quorum.
        let local_node_quorum_set: QuorumSet = {
            let inner_quorum_set_one = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(2), test_node_id(3), test_node_id(4)],
            );
            let inner_quorum_set_two = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(5), test_node_id(6), test_node_id(7)],
            );
            QuorumSet::new_with_inner_sets(2, vec![inner_quorum_set_one, inner_quorum_set_two])
        };
        let local_node_id = test_node_id(1);

        let node_2_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(3), test_node_id(4)]);
        let node_3_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(2), test_node_id(4)]);

        let topic = Topic::Prepare(PreparePayload::<u32> {
            B: Ballot::new(1, &[1234, 5678]),
            P: None,
            PP: None,
            CN: 0,
            HN: 0,
        });

        let mut msgs = HashMap::<NodeID, Msg<u32>>::default();
        msgs.insert(
            test_node_id(2),
            Msg::new(test_node_id(2), node_2_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(3),
            Msg::new(test_node_id(3), node_3_quorum_set, 1, topic),
        );

        let (node_ids, _) = local_node_quorum_set.findQuorum(
            &local_node_id,
            &msgs,
            FuncPredicate {
                test_fn: &|_msg| true,
            },
        );
        assert_eq!(node_ids, HashSet::from_iter(vec![]));
    }

    #[test]
    // findQuorum returns the correct set of nodes when there is a quorum
    fn test_has_quorum() {
        // Node 2 and 3 form a blocking set. Node 2, 3, 5, 6 form a quorum.
        let local_node_quorum_set: QuorumSet = {
            let inner_quorum_set_one = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(2), test_node_id(3), test_node_id(4)],
            );
            let inner_quorum_set_two = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(5), test_node_id(6), test_node_id(7)],
            );
            QuorumSet::new_with_inner_sets(2, vec![inner_quorum_set_one, inner_quorum_set_two])
        };
        let local_node_id = test_node_id(1);

        let node_2_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(3), test_node_id(4)]);
        let node_3_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(2), test_node_id(4)]);
        let node_5_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(6), test_node_id(7)]);
        let node_6_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(5), test_node_id(7)]);

        let topic = Topic::Prepare(PreparePayload::<u32> {
            B: Ballot::new(1, &[1234, 5678]),
            P: None,
            PP: None,
            CN: 0,
            HN: 0,
        });

        let mut msgs = HashMap::<NodeID, Msg<u32>>::default();
        msgs.insert(
            test_node_id(2),
            Msg::new(test_node_id(2), node_2_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(3),
            Msg::new(test_node_id(3), node_3_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(5),
            Msg::new(test_node_id(5), node_5_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(6),
            Msg::new(test_node_id(6), node_6_quorum_set, 1, topic),
        );

        let (node_ids, _) = local_node_quorum_set.findQuorum(
            &local_node_id,
            &msgs,
            FuncPredicate {
                test_fn: &|_msg| true,
            },
        );
        assert_eq!(
            node_ids,
            HashSet::from_iter(vec![
                test_node_id(2),
                test_node_id(3),
                test_node_id(5),
                test_node_id(6),
                test_node_id(1)
            ])
        );
    }

    #[test]
    // findQuorum returns an empty set when there is a quorum but the predicate returns false
    fn test_has_quorum_with_false_predicate() {
        // Node 2 and 3 form a blocking set. Node 2, 3, 5, 6 form a quorum.
        let local_node_quorum_set: QuorumSet = {
            let inner_quorum_set_one = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(2), test_node_id(3), test_node_id(4)],
            );
            let inner_quorum_set_two = QuorumSet::new_with_node_ids(
                2,
                vec![test_node_id(5), test_node_id(6), test_node_id(7)],
            );
            QuorumSet::new_with_inner_sets(2, vec![inner_quorum_set_one, inner_quorum_set_two])
        };
        let local_node_id = test_node_id(1);

        let node_2_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(3), test_node_id(4)]);
        let node_3_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(2), test_node_id(4)]);
        let node_5_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(6), test_node_id(7)]);
        let node_6_quorum_set =
            QuorumSet::new_with_node_ids(1, vec![test_node_id(5), test_node_id(7)]);

        let topic = Topic::Prepare(PreparePayload::<u32> {
            B: Ballot::new(1, &[1234, 5678]),
            P: None,
            PP: None,
            CN: 0,
            HN: 0,
        });

        let mut msgs = HashMap::<NodeID, Msg<u32>>::default();
        msgs.insert(
            test_node_id(2),
            Msg::new(test_node_id(2), node_2_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(3),
            Msg::new(test_node_id(3), node_3_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(5),
            Msg::new(test_node_id(5), node_5_quorum_set, 1, topic.clone()),
        );
        msgs.insert(
            test_node_id(6),
            Msg::new(test_node_id(6), node_6_quorum_set, 1, topic),
        );

        let (node_ids, _) = local_node_quorum_set.findQuorum(
            &local_node_id,
            &msgs,
            FuncPredicate {
                test_fn: &|msg| msg.sender_id != test_node_id(2),
            },
        );
        assert_eq!(node_ids, HashSet::from_iter(vec![]));
    }

    #[test]
    // Quorum set can be constructed with ResponderId
    fn test_blocking_set_with_responder_id() {
        // Quorum set by ResponderId, as employed by e.g. mobilecoind
        let mobilecoind_quorum_set: QuorumSet<ResponderId> = {
            let inner_quorum_set_one: QuorumSet<ResponderId> = QuorumSet::new_with_node_ids(
                2,
                vec![
                    test_node_id(2).responder_id,
                    test_node_id(3).responder_id,
                    test_node_id(4).responder_id,
                ],
            );
            let inner_quorum_set_two: QuorumSet<ResponderId> = QuorumSet::new_with_node_ids(
                2,
                vec![
                    test_node_id(5).responder_id,
                    test_node_id(6).responder_id,
                    test_node_id(7).responder_id,
                ],
            );
            QuorumSet::new_with_inner_sets(2, vec![inner_quorum_set_one, inner_quorum_set_two])
        };

        let topic = Topic::Prepare(PreparePayload::<u32> {
            B: Ballot::new(1, &[1234, 5678]),
            P: None,
            PP: None,
            CN: 0,
            HN: 0,
        });

        // Mimic polling_network_state.scp_network_state::push(Msg)
        let mut msgs = HashMap::<ResponderId, Msg<u32, ResponderId>>::default();
        msgs.insert(
            test_node_id(2).responder_id,
            Msg::new(
                test_node_id(2).responder_id,
                QuorumSet::empty(),
                1,
                topic.clone(),
            ),
        );
        msgs.insert(
            test_node_id(3).responder_id,
            Msg::new(test_node_id(3).responder_id, QuorumSet::empty(), 1, topic),
        );

        let responder_ids: HashSet<ResponderId> = HashSet::from_iter(vec![
            test_node_id(2).responder_id,
            test_node_id(3).responder_id,
            test_node_id(4).responder_id,
            test_node_id(5).responder_id,
            test_node_id(6).responder_id,
            test_node_id(7).responder_id,
        ]);

        let fp = FuncPredicate {
            test_fn: &|msg: &Msg<u32, ResponderId>| responder_ids.contains(&msg.sender_id),
        };

        let (node_ids, _) = mobilecoind_quorum_set.findBlockingSet(&msgs, fp);
        assert_eq!(
            node_ids,
            HashSet::from_iter(vec![
                test_node_id(2).responder_id,
                test_node_id(3).responder_id
            ])
        );
    }
}
