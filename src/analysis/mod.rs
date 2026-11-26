//! Calculs d'analyse produit sur un flux d'événements, indépendants de la base :
//! chaque moteur reçoit les événements session par session, dans l'ordre chronologique.

pub mod funnel;
pub mod paths;
