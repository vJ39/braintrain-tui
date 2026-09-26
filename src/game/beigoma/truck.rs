//! べーの軽トラの自動走行と、盤にかかるGの計算。
//!
//! Gは基本的に軽トラの速度・距離から運動方程式で求める。
//! - 巡航: 一定速度。ただし路面のガタガタとして常時小さな揺れ(CRUISE_VIBRATION_G)がかかる
//! - 信号のブレーキ: 気づいた時点の信号までの距離dと速度vから a = v^2 / (2d)
//! - 信号の発進: 巡航速度に戻るまで一定の目標加速度
//! - 障害物回避: t = d / v の間に横へlateral_distanceだけ動く a = 2 * lateral_distance / t^2
//! - 段差: 速度と段差の高さに比例した瞬間的な衝撃
//!
//! ここでのGは「軽トラの加速度」の向き(ブレーキ=後方向、発進=前方向、右へ避ける=右方向)で表す。
//! 盤上のベーゴマには慣性として逆向きの力がかかる(その換算は盤側が受け持つ)。

use std::time::Duration;

/// 重力加速度(m/s^2)。加速度をG単位に換算する
pub const GRAVITY: f64 = 9.8;
/// 巡航速度(m/s)。約40km/h
pub const CRUISE_SPEED: f64 = 11.0;
/// 信号で青になった時の発進加速度(m/s^2)
pub const LAUNCH_ACCEL: f64 = 2.5;
/// ブレーキの計算に使う信号までの距離の下限(m)。気づくのが極端に遅れても制動Gが発散しないようにする
pub const MIN_BRAKE_DISTANCE: f64 = 2.0;
/// 障害物回避にかける時間の下限(秒)。距離が極端に近くても横Gが発散しないようにする
pub const MIN_STEER_TIME: f64 = 0.3;
/// 段差の衝撃G = この係数 × 速度(m/s) × 段差の高さ(m)
pub const BUMP_G_FACTOR: f64 = 1.0;
/// 段差の衝撃が続く時間
pub const BUMP_DURATION: Duration = Duration::from_millis(200);
/// 信号が黄色に変わる、信号までの距離(m)。ここから仮想ドライバーが気づくまでの時間は信号ごとに決める
pub const YELLOW_DISTANCE: f64 = 30.0;
/// 赤信号で止まっている時間
pub const RED_WAIT: Duration = Duration::from_millis(2500);
/// 軽トラ視点に前方のイベントを予兆として出し始める距離(m)
pub const VISIBLE_DISTANCE: f64 = 45.0;
/// 巡航中、路面のガタガタとして常時かかる揺れの大きさ(G)。ブレーキ・段差・障害物回避の
/// Gより明確に小さく、キーで傾けなくてもベーゴマが体感できる程度に動く強さにする
pub const CRUISE_VIBRATION_G: f64 = 0.06;
/// 巡航中の揺れの波長(m)。前後・左右で異なる値にして、単調な往復に見えないようにする
const CRUISE_VIBRATION_WAVELENGTH_LONGITUDINAL: f64 = 2.6;
const CRUISE_VIBRATION_WAVELENGTH_LATERAL: f64 = 1.7;

/// 加速度(m/s^2)をG単位にする
pub fn to_g(accel: f64) -> f64 {
    accel / GRAVITY
}

/// 信号の手前で止まるのに必要な減速度(m/s^2)。a = v^2 / (2d)。
/// dはMIN_BRAKE_DISTANCEを下限にする(0や負でも発散しない)
pub fn braking_decel(speed: f64, distance: f64) -> f64 {
    let distance = distance.max(MIN_BRAKE_DISTANCE);
    speed * speed / (2.0 * distance)
}

/// 障害物を避けるのにかける時間(秒)。t = d / v(MIN_STEER_TIMEを下限にする)。
/// 止まっている(v<=0)なら避ける必要が無いのでNone
pub fn steer_time(speed: f64, distance: f64) -> Option<f64> {
    if speed <= 0.0 {
        return None;
    }
    Some((distance / speed).max(MIN_STEER_TIME))
}

/// 障害物を避けるのに必要な横加速度(m/s^2)。a = 2 * lateral_distance / t^2(t = d / v)
pub fn lateral_accel(speed: f64, distance: f64, lateral_distance: f64) -> f64 {
    match steer_time(speed, distance) {
        Some(t) => 2.0 * lateral_distance / (t * t),
        None => 0.0,
    }
}

/// 段差の衝撃(G)。速度と段差の高さに比例する
pub fn bump_g(speed: f64, height: f64) -> f64 {
    BUMP_G_FACTOR * speed * height
}

/// 盤にかかっているG(G単位)。軽トラの加速度の向きで、longitudinalは前方向が正、lateralは右方向が正
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct GForce {
    pub longitudinal: f64,
    pub lateral: f64,
}

impl GForce {
    /// Gの大きさ(前後・左右を合わせた大きさ)
    pub fn magnitude(&self) -> f64 {
        self.longitudinal.hypot(self.lateral)
    }
}

/// 障害物を避ける向き
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// 道路上のイベント。atはコース先頭からの距離(m)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RoadEvent {
    /// 段差。heightは段差の高さ(m)
    Bump { at: f64, height: f64 },
    /// 信号。YELLOW_DISTANCE手前で黄色に変わり、notice_delay秒後に仮想ドライバーが気づいてブレーキを踏む
    Signal { at: f64, notice_delay: f64 },
    /// 前方の障害物。react_distance手前で気づき、sideへlateral_distanceだけ避ける
    Obstacle {
        at: f64,
        react_distance: f64,
        lateral_distance: f64,
        side: Side,
    },
}

/// 信号の色(軽トラ視点の予兆表示用)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalLight {
    Green,
    Yellow,
    Red,
}

/// 軽トラ視点に出す、前方のイベントの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpcomingKind {
    Bump,
    Signal(SignalLight),
    Obstacle(Side),
}

/// 軽トラ視点に出す、前方のイベントとそこまでの距離(m)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Upcoming {
    pub kind: UpcomingKind,
    pub distance: f64,
}

/// 決まったコース(1周の長さ, イベント)。毎回同じ順番・同じ強さのGが来るので、繰り返しプレイで上達できる。
/// 1周走り切ったら先頭から繰り返す。ゴールまで数秒〜十数秒で届く盤なので、序盤からGが来るように並べる
/// (巡航11m/sで、段差約3秒後・最初のブレーキ約7秒後。後半ほど強いGになる)
pub const COURSE_LENGTH: f64 = 400.0;
pub const COURSE: [RoadEvent; 6] = [
    RoadEvent::Bump {
        at: 35.0,
        height: 0.03,
    },
    RoadEvent::Signal {
        at: 100.0,
        notice_delay: 0.9,
    },
    RoadEvent::Obstacle {
        at: 170.0,
        react_distance: 10.0,
        lateral_distance: 1.5,
        side: Side::Right,
    },
    RoadEvent::Bump {
        at: 220.0,
        height: 0.08,
    },
    RoadEvent::Signal {
        at: 300.0,
        notice_delay: 2.2,
    },
    RoadEvent::Obstacle {
        at: 370.0,
        react_distance: 6.5,
        lateral_distance: 1.5,
        side: Side::Left,
    },
];

/// 軽トラの走り方の種類(軽トラ視点のスキール音等、外部から走り方の変化を検知するのに使う)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionKind {
    Cruise,
    Noticing,
    Braking,
    Stopped,
    Launching,
    Steering,
}

/// 軽トラの走り方
#[derive(Debug, Clone, Copy, PartialEq)]
enum Motion {
    /// 巡航速度で走っている
    Cruise,
    /// 信号が黄色になり、仮想ドライバーが気づくまでの間(まだ巡航速度)。signal_atは信号の位置
    Noticing { signal_at: f64, remaining: f64 },
    /// 信号の手前で止まるためにブレーキ中
    Braking { signal_at: f64, decel: f64 },
    /// 赤信号で停止中
    Stopped { signal_at: f64, remaining: f64 },
    /// 青信号で発進し、巡航速度へ加速中
    Launching,
    /// 障害物を避けるため操舵中。accelは右方向が正の横加速度
    Steering { accel: f64, remaining: f64 },
}

/// 軽トラの走行シミュレーション
#[derive(Debug, Clone)]
pub struct Truck {
    course: Vec<RoadEvent>,
    course_length: f64,
    /// 走った距離(m)
    position: f64,
    /// 現在の速度(m/s)
    speed: f64,
    motion: Motion,
    /// 次に処理するイベント(courseの添字)と、それが何周目か
    next_event: usize,
    lap: u32,
    /// 段差の衝撃(G, 残り時間)
    bump: Option<(f64, Duration)>,
}

impl Truck {
    /// 決まったコース(COURSE)を巡航速度で走り始める
    pub fn new() -> Self {
        Self::with_course(COURSE.to_vec(), COURSE_LENGTH)
    }

    /// コースを指定して作る(テストで個別のイベントを確かめるため)。courseが空ならイベントは起きない
    pub fn with_course(course: Vec<RoadEvent>, course_length: f64) -> Self {
        Self {
            course,
            course_length,
            position: 0.0,
            speed: CRUISE_SPEED,
            motion: Motion::Cruise,
            next_event: 0,
            lap: 0,
            bump: None,
        }
    }

    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// いまの走り方の種類。ブレーキ・発進・操舵が新しく始まったかを外部で検知するのに使う
    pub fn motion_kind(&self) -> MotionKind {
        match self.motion {
            Motion::Cruise => MotionKind::Cruise,
            Motion::Noticing { .. } => MotionKind::Noticing,
            Motion::Braking { .. } => MotionKind::Braking,
            Motion::Stopped { .. } => MotionKind::Stopped,
            Motion::Launching => MotionKind::Launching,
            Motion::Steering { .. } => MotionKind::Steering,
        }
    }

    /// 経過時間を進める
    pub fn update(&mut self, dt: Duration) {
        let secs = dt.as_secs_f64();
        if let Some((g, remaining)) = self.bump {
            let remaining = remaining.saturating_sub(dt);
            self.bump = (!remaining.is_zero()).then_some((g, remaining));
        }
        self.advance_motion(secs);
        self.position += self.speed * secs;
        self.brake_if_noticed();
        self.trigger_events();
    }

    /// 走り方ごとに速度・残り時間を進め、終わったら次の走り方へ移る
    fn advance_motion(&mut self, secs: f64) {
        self.motion = match self.motion {
            Motion::Cruise => Motion::Cruise,
            Motion::Noticing {
                signal_at,
                remaining,
            } => Motion::Noticing {
                signal_at,
                remaining: remaining - secs,
            },
            Motion::Braking { signal_at, decel } => {
                self.speed -= decel * secs;
                if self.speed <= 0.0 {
                    self.speed = 0.0;
                    Motion::Stopped {
                        signal_at,
                        remaining: RED_WAIT.as_secs_f64(),
                    }
                } else {
                    Motion::Braking { signal_at, decel }
                }
            }
            Motion::Stopped {
                signal_at,
                remaining,
            } => {
                let remaining = remaining - secs;
                if remaining <= 0.0 {
                    Motion::Launching
                } else {
                    Motion::Stopped {
                        signal_at,
                        remaining,
                    }
                }
            }
            Motion::Launching => {
                self.speed += LAUNCH_ACCEL * secs;
                if self.speed >= CRUISE_SPEED {
                    self.speed = CRUISE_SPEED;
                    Motion::Cruise
                } else {
                    Motion::Launching
                }
            }
            Motion::Steering { accel, remaining } => {
                let remaining = remaining - secs;
                if remaining <= 0.0 {
                    Motion::Cruise
                } else {
                    Motion::Steering { accel, remaining }
                }
            }
        };
    }

    /// 仮想ドライバーが信号に気づいたら、その時点の信号までの距離からブレーキの減速度を決める
    fn brake_if_noticed(&mut self) {
        if let Motion::Noticing {
            signal_at,
            remaining,
        } = self.motion
        {
            if remaining <= 0.0 {
                let decel = braking_decel(self.speed, signal_at - self.position);
                self.motion = Motion::Braking { signal_at, decel };
            }
        }
    }

    /// 次のイベントの位置まで来ていれば始める(同じステップで複数来ることもある)
    fn trigger_events(&mut self) {
        while let Some(event) = self.course.get(self.next_event).copied() {
            let lap_start = f64::from(self.lap) * self.course_length;
            let cruising = self.motion == Motion::Cruise;
            match event {
                RoadEvent::Bump { at, height } if self.position >= lap_start + at => {
                    self.bump = Some((bump_g(self.speed, height), BUMP_DURATION));
                }
                RoadEvent::Signal { at, notice_delay }
                    if cruising && self.position >= lap_start + at - YELLOW_DISTANCE =>
                {
                    self.motion = Motion::Noticing {
                        signal_at: lap_start + at,
                        remaining: notice_delay,
                    };
                }
                RoadEvent::Obstacle {
                    at,
                    react_distance,
                    lateral_distance,
                    side,
                } if cruising && self.position >= lap_start + at - react_distance => {
                    let distance = lap_start + at - self.position;
                    if let Some(time) = steer_time(self.speed, distance) {
                        let accel = lateral_accel(self.speed, distance, lateral_distance);
                        let sign = match side {
                            Side::Right => 1.0,
                            Side::Left => -1.0,
                        };
                        self.motion = Motion::Steering {
                            accel: sign * accel,
                            remaining: time,
                        };
                    }
                }
                _ => return,
            }
            self.next_event += 1;
            if self.next_event >= self.course.len() {
                self.next_event = 0;
                self.lap += 1;
            }
        }
    }

    /// いま盤にかかっているG
    pub fn current_g(&self) -> GForce {
        let mut g = GForce::default();
        match self.motion {
            Motion::Braking { decel, .. } => g.longitudinal = -to_g(decel),
            Motion::Launching => g.longitudinal = to_g(LAUNCH_ACCEL),
            Motion::Steering { accel, .. } => g.lateral = to_g(accel),
            // 段差(bump)が起きている間は、その衝撃の方が支配的なので巡航の揺れは足さない
            Motion::Cruise if self.bump.is_none() => g = self.cruise_vibration(),
            Motion::Cruise | Motion::Noticing { .. } | Motion::Stopped { .. } => {}
        }
        // 段差の突き上げは、盤面上は後方向の揺れとして表す
        if let Some((bump, _)) = self.bump {
            g.longitudinal -= bump;
        }
        g
    }

    /// 巡航中、路面のガタガタとして常時かかる小さな揺れ。走行距離を種にした周期関数で、
    /// 前後・左右で異なる波長のsin波を合成し、単調な往復に見えないようにする
    fn cruise_vibration(&self) -> GForce {
        let phase = |wavelength: f64| self.position / wavelength * std::f64::consts::TAU;
        GForce {
            longitudinal: (phase(CRUISE_VIBRATION_WAVELENGTH_LONGITUDINAL).sin()
                + (phase(CRUISE_VIBRATION_WAVELENGTH_LONGITUDINAL) * 2.3).sin())
                * 0.5
                * CRUISE_VIBRATION_G,
            lateral: (phase(CRUISE_VIBRATION_WAVELENGTH_LATERAL).sin()
                + (phase(CRUISE_VIBRATION_WAVELENGTH_LATERAL) * 1.7).sin())
                * 0.5
                * CRUISE_VIBRATION_G,
        }
    }

    /// 前方(VISIBLE_DISTANCE以内)の次のイベント。軽トラ視点の予兆表示に使う
    pub fn upcoming(&self) -> Option<Upcoming> {
        let signal = |light: SignalLight, signal_at: f64| {
            Some(Upcoming {
                kind: UpcomingKind::Signal(light),
                distance: (signal_at - self.position).max(0.0),
            })
        };
        match self.motion {
            Motion::Noticing { signal_at, .. } => return signal(SignalLight::Yellow, signal_at),
            Motion::Braking { signal_at, .. } | Motion::Stopped { signal_at, .. } => {
                return signal(SignalLight::Red, signal_at)
            }
            _ => {}
        }
        let event = self.course.get(self.next_event)?;
        let lap_start = f64::from(self.lap) * self.course_length;
        let (at, kind) = match *event {
            RoadEvent::Bump { at, .. } => (at, UpcomingKind::Bump),
            RoadEvent::Signal { at, .. } => (at, UpcomingKind::Signal(SignalLight::Green)),
            RoadEvent::Obstacle { at, side, .. } => (at, UpcomingKind::Obstacle(side)),
        };
        let distance = (lap_start + at - self.position).max(0.0);
        (distance <= VISIBLE_DISTANCE).then_some(Upcoming { kind, distance })
    }
}

impl Default for Truck {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: Duration = Duration::from_millis(10);

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// 条件を満たすまでSTEPずつ進める(最大limit秒)。満たせなければpanic
    fn run_until(truck: &mut Truck, limit: f64, mut done: impl FnMut(&Truck) -> bool) {
        let mut t = 0.0;
        while !done(truck) {
            truck.update(STEP);
            t += STEP.as_secs_f64();
            assert!(t < limit, "{limit}秒以内に条件を満たさなかった");
        }
    }

    fn is_braking(truck: &Truck) -> bool {
        matches!(truck.motion, Motion::Braking { .. })
    }

    #[test]
    fn motion_kind_reflects_braking_stopped_launching_and_steering() {
        let mut truck = signal_course(0.5);
        assert_eq!(truck.motion_kind(), MotionKind::Cruise);
        run_until(&mut truck, 10.0, is_braking);
        assert_eq!(truck.motion_kind(), MotionKind::Braking);
        run_until(&mut truck, 10.0, |t| {
            matches!(t.motion, Motion::Stopped { .. })
        });
        assert_eq!(truck.motion_kind(), MotionKind::Stopped);
        run_until(&mut truck, 10.0, |t| matches!(t.motion, Motion::Launching));
        assert_eq!(truck.motion_kind(), MotionKind::Launching);

        let mut truck = Truck::with_course(
            vec![RoadEvent::Obstacle {
                at: 50.0,
                react_distance: 10.0,
                lateral_distance: 1.5,
                side: Side::Right,
            }],
            1000.0,
        );
        run_until(&mut truck, 10.0, |t| {
            matches!(t.motion, Motion::Steering { .. })
        });
        assert_eq!(truck.motion_kind(), MotionKind::Steering);
    }

    fn signal_course(notice_delay: f64) -> Truck {
        Truck::with_course(
            vec![RoadEvent::Signal {
                at: 60.0,
                notice_delay,
            }],
            1000.0,
        )
    }

    // --- 式そのもの ---

    #[test]
    fn braking_decel_follows_v_squared_over_2d() {
        assert!(approx(
            braking_decel(11.0, 20.0),
            11.0 * 11.0 / (2.0 * 20.0)
        ));
        assert!(approx(braking_decel(8.0, 10.0), 64.0 / 20.0));
    }

    #[test]
    fn braking_decel_grows_as_the_distance_gets_shorter() {
        // 気づくのが遅い(dが小さい)ほど制動Gが大きい
        let far = braking_decel(CRUISE_SPEED, 20.0);
        let near = braking_decel(CRUISE_SPEED, 5.0);
        assert!(near > far);
        assert!(braking_decel(CRUISE_SPEED, 3.0) > near);
    }

    #[test]
    fn braking_decel_does_not_diverge_for_tiny_distances() {
        let clamped = CRUISE_SPEED * CRUISE_SPEED / (2.0 * MIN_BRAKE_DISTANCE);
        for d in [MIN_BRAKE_DISTANCE, 1.0, 0.1, 0.0, -5.0] {
            let a = braking_decel(CRUISE_SPEED, d);
            assert!(a.is_finite(), "d={d}");
            assert!(approx(a, clamped), "d={d}: 下限の距離で計算する");
        }
    }

    #[test]
    fn braking_g_matches_the_spec_rough_numbers() {
        // 40km/h(約11m/s)で20m手前なら約0.3G、5mまで遅れると約1.2G
        let v = 40.0 / 3.6;
        assert!((to_g(braking_decel(v, 20.0)) - 0.31).abs() < 0.02);
        assert!((to_g(braking_decel(v, 5.0)) - 1.26).abs() < 0.02);
    }

    #[test]
    fn lateral_accel_follows_the_uniform_acceleration_formula() {
        let (v, d, l) = (11.0, 10.0, 1.5);
        let t: f64 = d / v;
        assert!(approx(steer_time(v, d).unwrap(), t));
        assert!(approx(lateral_accel(v, d, l), 2.0 * l / (t * t)));
    }

    #[test]
    fn lateral_accel_grows_when_closer_or_faster() {
        let base = lateral_accel(11.0, 12.0, 1.5);
        assert!(lateral_accel(11.0, 6.0, 1.5) > base, "近いほど大きい");
        assert!(lateral_accel(15.0, 12.0, 1.5) > base, "速いほど大きい");
    }

    #[test]
    fn lateral_accel_is_bounded_and_zero_when_stopped() {
        let bounded = 2.0 * 1.5 / (MIN_STEER_TIME * MIN_STEER_TIME);
        assert!(
            approx(lateral_accel(11.0, 0.0, 1.5), bounded),
            "時間の下限で発散しない"
        );
        assert!(approx(lateral_accel(11.0, 0.1, 1.5), bounded));
        assert_eq!(steer_time(0.0, 10.0), None);
        assert_eq!(lateral_accel(0.0, 10.0, 1.5), 0.0, "止まっていれば避けない");
    }

    #[test]
    fn bump_g_is_proportional_to_speed() {
        let slow = bump_g(5.0, 0.05);
        let fast = bump_g(10.0, 0.05);
        assert!(approx(fast, slow * 2.0));
        assert!(approx(bump_g(11.0, 0.05), BUMP_G_FACTOR * 11.0 * 0.05));
        assert_eq!(bump_g(0.0, 0.05), 0.0);
    }

    #[test]
    fn to_g_divides_by_gravity() {
        assert!(approx(to_g(GRAVITY), 1.0));
        assert!(approx(to_g(4.9), 0.5));
    }

    #[test]
    fn g_magnitude_combines_both_axes() {
        let g = GForce {
            longitudinal: 0.3,
            lateral: -0.4,
        };
        assert!(approx(g.magnitude(), 0.5));
        assert_eq!(GForce::default().magnitude(), 0.0);
    }

    // --- 走行シミュレーション ---

    #[test]
    fn cruising_has_small_vibration_instead_of_zero_g() {
        // 巡航中もキーで傾けなくてもベーゴマが動くよう、路面のガタガタとして
        // 常時小さなGがかかる(完全なゼロではない)
        let mut truck = Truck::with_course(Vec::new(), 1000.0);
        let mut saw_nonzero = false;
        for _ in 0..500 {
            truck.update(STEP);
            let g = truck.current_g();
            if g != GForce::default() {
                saw_nonzero = true;
            }
            assert!(
                g.magnitude() < CRUISE_VIBRATION_G * 2.0,
                "巡航中の揺れは他のイベントより明確に小さい: {g:?}"
            );
            assert!(approx(truck.speed(), CRUISE_SPEED));
        }
        assert!(saw_nonzero, "巡航中も揺れが発生すること");
        assert!(truck.upcoming().is_none());
    }

    #[test]
    fn cruise_vibration_changes_as_the_truck_moves() {
        let mut truck = Truck::with_course(Vec::new(), 1000.0);
        let first = truck.current_g();
        for _ in 0..50 {
            truck.update(STEP);
        }
        let later = truck.current_g();
        assert_ne!(first, later, "走行距離が変われば揺れも変わる(固定値ではない)");
    }

    #[test]
    fn signal_braking_g_is_v_squared_over_2d_at_the_moment_of_noticing() {
        let mut truck = signal_course(0.5);
        run_until(&mut truck, 10.0, is_braking);
        // 気づいた瞬間の信号までの距離から減速度を求めている
        let distance = 60.0 - truck.position;
        let expected = to_g(braking_decel(CRUISE_SPEED, distance));
        let g = truck.current_g();
        assert!((g.longitudinal + expected).abs() < 1e-9, "ブレーキは後方向");
        assert_eq!(g.lateral, 0.0);
    }

    #[test]
    fn noticing_the_signal_later_gives_a_larger_braking_g() {
        let brake_g = |delay: f64| {
            let mut truck = signal_course(delay);
            run_until(&mut truck, 10.0, is_braking);
            truck.current_g().magnitude()
        };
        let early = brake_g(0.5);
        let late = brake_g(2.0);
        assert!(late > early, "遅く気づくほど大きい: {early} < {late}");
    }

    #[test]
    fn noticing_after_passing_the_signal_still_gives_a_finite_braking_g() {
        // 気づくのが極端に遅れて信号を越えていても、下限の距離で計算して発散しない
        let mut truck = signal_course(10.0);
        run_until(&mut truck, 20.0, is_braking);
        let g = truck.current_g().magnitude();
        assert!(g.is_finite());
        assert!(approx(
            g,
            to_g(braking_decel(CRUISE_SPEED, MIN_BRAKE_DISTANCE))
        ));
    }

    #[test]
    fn signal_turns_yellow_before_the_driver_notices() {
        let mut truck = signal_course(1.0);
        run_until(&mut truck, 10.0, |t| {
            matches!(t.motion, Motion::Noticing { .. })
        });
        assert!(
            truck.current_g().magnitude() < CRUISE_VIBRATION_G * 2.0,
            "気づくまではまだ巡航(揺れの範囲内)"
        );
        let upcoming = truck.upcoming().expect("信号が見えている");
        assert_eq!(upcoming.kind, UpcomingKind::Signal(SignalLight::Yellow));
        assert!(upcoming.distance <= YELLOW_DISTANCE + 1e-9);
    }

    #[test]
    fn truck_stops_waits_on_red_and_launches_with_the_target_accel() {
        let mut truck = signal_course(0.5);
        run_until(&mut truck, 10.0, is_braking);
        run_until(&mut truck, 10.0, |t| {
            matches!(t.motion, Motion::Stopped { .. })
        });
        assert_eq!(truck.speed(), 0.0);
        assert_eq!(truck.current_g(), GForce::default(), "停止中はGなし");
        assert_eq!(
            truck.upcoming().map(|u| u.kind),
            Some(UpcomingKind::Signal(SignalLight::Red))
        );
        run_until(&mut truck, 10.0, |t| matches!(t.motion, Motion::Launching));
        // 発進のGは一定の目標加速度(前方向)
        let g = truck.current_g();
        assert!(approx(g.longitudinal, to_g(LAUNCH_ACCEL)));
        let before = truck.speed();
        truck.update(STEP);
        assert!(approx(
            truck.speed() - before,
            LAUNCH_ACCEL * STEP.as_secs_f64()
        ));
        run_until(&mut truck, 10.0, |t| matches!(t.motion, Motion::Cruise));
        assert!(approx(truck.speed(), CRUISE_SPEED), "巡航速度に戻る");
        assert!(truck.current_g().magnitude() < CRUISE_VIBRATION_G * 2.0);
    }

    #[test]
    fn braking_stops_near_the_signal() {
        let mut truck = signal_course(0.5);
        run_until(&mut truck, 10.0, |t| {
            matches!(t.motion, Motion::Stopped { .. })
        });
        assert!(
            (truck.position - 60.0).abs() < 1.0,
            "信号の手前で止まる: {}",
            truck.position
        );
    }

    #[test]
    fn obstacle_avoidance_gives_lateral_g_from_distance_and_speed() {
        for (side, sign) in [(Side::Right, 1.0), (Side::Left, -1.0)] {
            let mut truck = Truck::with_course(
                vec![RoadEvent::Obstacle {
                    at: 50.0,
                    react_distance: 10.0,
                    lateral_distance: 1.5,
                    side,
                }],
                1000.0,
            );
            run_until(&mut truck, 10.0, |t| {
                matches!(t.motion, Motion::Steering { .. })
            });
            let distance = 50.0 - truck.position;
            let expected = to_g(lateral_accel(CRUISE_SPEED, distance, 1.5));
            let g = truck.current_g();
            assert!((g.lateral - sign * expected).abs() < 1e-9, "{side:?}");
            assert_eq!(g.longitudinal, 0.0);
            // 避け終わったら巡航に戻る(t = d / v の間だけ続く)
            let t = steer_time(CRUISE_SPEED, distance).unwrap();
            let mut elapsed = 0.0;
            run_until(&mut truck, 10.0, |t| {
                elapsed += STEP.as_secs_f64();
                matches!(t.motion, Motion::Cruise)
            });
            assert!((elapsed - t).abs() < 0.05, "{side:?}: {elapsed} vs {t}");
            assert!(truck.current_g().magnitude() < CRUISE_VIBRATION_G * 2.0);
        }
    }

    #[test]
    fn bump_gives_a_short_longitudinal_jolt_scaled_by_speed() {
        let mut truck = Truck::with_course(
            vec![RoadEvent::Bump {
                at: 30.0,
                height: 0.05,
            }],
            1000.0,
        );
        // 巡航の揺れ(CRUISE_VIBRATION_G)よりずっと大きいので、閾値超えでbump本体と区別できる
        run_until(&mut truck, 10.0, |t| {
            t.current_g().magnitude() > CRUISE_VIBRATION_G * 2.0
        });
        let g = truck.current_g();
        assert!(approx(g.longitudinal.abs(), bump_g(CRUISE_SPEED, 0.05)));
        assert_eq!(g.lateral, 0.0);
        // 衝撃はBUMP_DURATIONで消える(消えた後は巡航の揺れの範囲に戻る)
        let mut elapsed = 0.0;
        run_until(&mut truck, 5.0, |t| {
            elapsed += STEP.as_secs_f64();
            t.current_g().magnitude() < CRUISE_VIBRATION_G * 2.0
        });
        assert!((elapsed - BUMP_DURATION.as_secs_f64()).abs() < 0.03);
    }

    #[test]
    fn upcoming_shows_the_next_event_within_the_visible_distance() {
        let mut truck = Truck::with_course(
            vec![RoadEvent::Bump {
                at: 100.0,
                height: 0.05,
            }],
            1000.0,
        );
        assert!(truck.upcoming().is_none(), "遠いうちは見えない");
        run_until(&mut truck, 20.0, |t| t.upcoming().is_some());
        let upcoming = truck.upcoming().unwrap();
        assert_eq!(upcoming.kind, UpcomingKind::Bump);
        assert!(upcoming.distance <= VISIBLE_DISTANCE && upcoming.distance > 0.0);
    }

    #[test]
    fn course_repeats_after_one_lap() {
        let mut truck = Truck::with_course(
            vec![RoadEvent::Bump {
                at: 5.0,
                height: 0.05,
            }],
            20.0,
        );
        let mut jolts = 0;
        let mut was_jolting = false;
        for _ in 0..600 {
            truck.update(STEP);
            let jolting = truck.current_g().magnitude() > CRUISE_VIBRATION_G * 2.0;
            if jolting && !was_jolting {
                jolts += 1;
            }
            was_jolting = jolting;
        }
        // 6秒で66m=3周ちょっと走るので、段差は周回ごとに来る
        assert!(jolts >= 3, "周回ごとに段差が来る: {jolts}");
    }

    #[test]
    fn default_course_produces_every_kind_of_g_and_stays_finite() {
        let mut truck = Truck::new();
        let (mut braked, mut launched, mut steered, mut bumped) = (false, false, false, false);
        let mut max_g: f64 = 0.0;
        for _ in 0..6000 {
            truck.update(STEP);
            let g = truck.current_g();
            assert!(g.longitudinal.is_finite() && g.lateral.is_finite());
            max_g = max_g.max(g.magnitude());
            match truck.motion {
                Motion::Braking { .. } => braked = true,
                Motion::Launching => launched = true,
                Motion::Steering { .. } => steered = true,
                _ => {}
            }
            if truck.bump.is_some() {
                bumped = true;
            }
        }
        assert!(braked && launched && steered && bumped);
        // 強いG(吹っ飛びの目安0.8G)を超える場面もある
        assert!(max_g > 0.8, "max_g={max_g}");
        assert!(max_g < 5.0, "max_g={max_g}");
    }
}
