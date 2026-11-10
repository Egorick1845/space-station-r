use super::*;

#[test]
fn group_split_matches_damage_specifier() {
    let brute = DamageSpecifier::group(DamageGroup::Brute, -30.0);
    assert_eq!(brute.get(DamageType::Blunt), -10.0);
    assert_eq!(brute.get(DamageType::Slash), -10.0);
    assert_eq!(brute.get(DamageType::Piercing), -10.0);
    // 10 / 3: доли с шагом 0.01, остаток округления не теряется.
    let odd = DamageSpecifier::group(DamageGroup::Brute, 10.0);
    assert!((odd.get(DamageType::Blunt) - 3.33).abs() < 1e-4);
    for damage_type in DamageGroup::Brute.types() {
        assert!((odd.get(*damage_type) - 10.0 / 3.0).abs() <= 0.011);
    }
    assert!((odd.total() - 10.0).abs() < 1e-4);
}

#[test]
fn thresholds_move_between_states() {
    let mut mob = Damageable::default();
    mob.change(&DamageSpecifier::of(DamageType::Blunt, 99.0));
    assert_eq!(mob.update_state(), None);
    mob.change(&DamageSpecifier::of(DamageType::Slash, 1.0));
    assert_eq!(mob.update_state(), Some((MobState::Alive, MobState::Critical)));
    assert!(mob.incapacitated());
    mob.change(&DamageSpecifier::of(DamageType::Blunt, -10.0));
    assert_eq!(mob.update_state(), Some((MobState::Critical, MobState::Alive)));
    mob.change(&DamageSpecifier::of(DamageType::Heat, 200.0));
    assert_eq!(mob.update_state(), Some((MobState::Alive, MobState::Dead)));
    // AllowRevives = false: мёртвого лечение не поднимает.
    mob.change(&DamageSpecifier::of(DamageType::Heat, -200.0));
    assert_eq!(mob.update_state(), None);
    assert_eq!(mob.state, MobState::Dead);
    mob.rejuvenate();
    assert!(mob.is_alive());
    assert_eq!(mob.total(), 0.0);
}

#[test]
fn damage_never_goes_negative_and_structural_is_ignored() {
    let mut mob = Damageable::default();
    mob.change(&DamageSpecifier::of(DamageType::Blunt, 5.0));
    let applied = mob.change(&DamageSpecifier::group(DamageGroup::Brute, -30.0));
    assert_eq!(applied.get(DamageType::Blunt), -5.0);
    assert_eq!(applied.total(), -5.0);
    assert_eq!(mob.total(), 0.0);
    mob.change(&DamageSpecifier::of(DamageType::Structural, 50.0));
    assert_eq!(mob.total(), 0.0);
}

#[test]
fn alert_severity_follows_mob_threshold_system() {
    let mut mob = Damageable::default();
    assert_eq!(mob.alert(), HealthAlert::Health(0));
    mob.change(&DamageSpecifier::of(DamageType::Blunt, 50.0));
    assert_eq!(mob.alert(), HealthAlert::Health(2));
    mob.change(&DamageSpecifier::of(DamageType::Blunt, 40.0));
    assert_eq!(mob.alert(), HealthAlert::Health(4));
    mob.change(&DamageSpecifier::of(DamageType::Blunt, 10.0));
    mob.update_state();
    assert_eq!(mob.alert(), HealthAlert::Crit);
}

#[test]
fn healing_items_follow_prototypes() {
    let brute = healing_for("Brutepack").unwrap();
    assert_eq!(brute.damage.total(), -30.0);
    let ointment = healing_for("Ointment").unwrap();
    assert_eq!(ointment.damage.get(DamageType::Caustic), -5.0);
    assert!(healing_for("Crowbar").is_none());

    let mut mob = Damageable::default();
    assert!(!brute.has_damage(&mob));
    mob.change(&DamageSpecifier::of(DamageType::Heat, 10.0));
    assert!(!brute.has_damage(&mob), "набор для ушибов ожоги не лечит");
    assert!(ointment.has_damage(&mob));
    assert_eq!(scaled_healing_penalty(0.0, 2.0), 1.0);
    assert_eq!(scaled_healing_penalty(50.0, 2.0), 1.5);
    assert_eq!(ointment.delay_for(false, &mob), 2.0);
}

#[test]
fn passive_regen_only_heals_damaged_brute_types() {
    let mut mob = Damageable::default();
    mob.change(&DamageSpecifier::of(DamageType::Blunt, 10.0));
    let regen = mob.passive_regen();
    assert_eq!(regen.get(DamageType::Blunt), PASSIVE_BRUTE);
    assert_eq!(regen.get(DamageType::Slash), 0.0);
    mob.change(&DamageSpecifier::of(DamageType::Blunt, 15.0));
    assert_eq!(mob.passive_regen().total(), 0.0, "выше damageCap 20");
}

#[test]
fn crit_breathing_reaches_asphyxiation_after_three_cycles() {
    let mut saturation = respirator::MAX_SATURATION;
    let mut damage = Vec::new();
    for _ in 0..4 {
        let (next, hurt) = respirator::cycle(saturation, false);
        saturation = next;
        damage.push(hurt);
    }
    assert_eq!(damage, vec![0.0, 0.0, 1.0, 1.0]);
    assert_eq!(saturation, respirator::MIN_SATURATION);
    assert_eq!(respirator::cycle(saturation, true), (5.0, -1.0));
}

#[test]
fn progress_color_goes_red_to_green() {
    let start = progress_color(0.0);
    assert!(start[0] > 0.7 && start[1] < 0.01);
    assert_eq!(progress_color(1.0), [0.0, 1.0, 0.0]);
}

#[test]
fn slow_on_damage_thresholds() {
    assert_eq!(slow_on_damage(59.0), 1.0);
    assert_eq!(slow_on_damage(60.0), 0.7);
    assert_eq!(slow_on_damage(80.0), 0.5);
}
