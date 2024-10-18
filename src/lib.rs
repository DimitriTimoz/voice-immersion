#![allow(clippy::precedence)]

use std::sync::{Arc, RwLock};

use assert_no_alloc::*;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use crossbeam_channel::{Receiver, Sender};
use fundsp::hacker::*;
use hacker32::sine;
use nalgebra::Vector3;

#[cfg(debug_assertions)] // required when disable_release is set (default)
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

const SOUND_SPEED: f32 = 343.0;
pub const HEAD_RADIUS: f32 = 0.10;
const UP_VECTOR: Vector3<f32> = Vector3::new(0.0, 1.0, 0.0);

#[derive(Debug, Clone)]
pub struct InAnotherRoom {
    pub wall_width: f32,
    pub wall_attenuation_factor: f32,
    pub cutoff_frequency: f32,
}

#[derive(Debug, Clone)]
pub struct SourceInfo {
    pub relative_position: Vector3<f32>,
    pub direction: Vector3<f32>,
    pub room: Option<InAnotherRoom>,
}

impl Default for SourceInfo {
    fn default() -> Self {
        SourceInfo {
            relative_position: Vector3::new(0.0, 0.0, 0.0),
            direction: Vector3::new(1.0, 0.0, 0.0),
            room: None,
        }
    }
}

#[derive(Clone)]
pub struct InputNode {
    receiver: Receiver<(f32, f32)>,
}

impl InputNode {
    pub fn new(receiver: Receiver<(f32, f32)>) -> Self {
        InputNode { receiver }
    }
}

impl AudioNode for InputNode {
    const ID: u64 = 87;
    type Inputs = U0;
    type Outputs = U2;

    #[inline]
    fn tick(&mut self, _input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        let (left, right) = self.receiver.try_recv().unwrap_or((0.0, 0.0));
        [left, right].into()
    }
}

pub fn run_in<T>(device: &cpal::Device, config: &cpal::StreamConfig, sender: Sender<(f32, f32)>)
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| read_data(data, channels, sender.clone()),
        err_fn,
        None,
    );

    if let Ok(stream) = stream {
        if let Ok(()) = stream.play() {
            std::mem::forget(stream);
        }
    }
    println!("Input stream built.");
}

fn read_data<T>(input: &[T], channels: usize, sender: Sender<(f32, f32)>)
where
    T: SizedSample,
    f32: FromSample<T>,
{
    for frame in input.chunks(channels) {
        let mut left = 0.0;
        let mut right = 0.0;
        for (channel, sample) in frame.iter().enumerate() {
            if channel & 1 == 0 {
                left = sample.to_sample::<f32>();
            } else {
                right = sample.to_sample::<f32>();
            }
        }
        if let Ok(()) = sender.try_send((left, right)) {}
    }
}

pub fn room_amplitude_factor(room: Option<InAnotherRoom>) -> f32 {
    if let Some(room) = room {
        let wall_attenuation = room.wall_attenuation_factor;
        let wall_width = room.wall_width;

        (-(wall_width) * wall_attenuation).exp()
    } else {
        1.0
    }
}

pub fn run_out<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    receiver: Receiver<(f32, f32)>,
    wave: Option<fundsp::wave::Wave>,
    source_info: Arc<RwLock<SourceInfo>>,
) -> Result<(), anyhow::Error>
where
    T: SizedSample + FromSample<f32> + Send,
{
    //let input = An(InputNode::new(receiver));
    #[cfg(not(feature = "mic"))]
    let wave = wave.unwrap();
    #[cfg(not(feature = "mic"))]
    let input = WavePlayer::new(&Arc::new(wave.clone()), 0, 0, wave.length(), Some(0));

    #[cfg(feature = "mic")]
    let input = InputNode::new(receiver);

    let sample_rate = config.sample_rate.0 as f64;
    let channels = config.channels as usize;
    let amplitude: Shared = shared(1.0);
    let (left_amp, right_amp) = (shared(1.0), shared(1.0));

    let mut net = Net::new(1, 2);
    //let mut net = Net::wrap(Box::new(An(input)));
    let input_node = net.push(Box::new(sine()));
    net.set_sample_rate(sample_rate);
    net.chain(Box::new(tick() * (var(&amplitude) >> follow(0.1))));

    let (material_filter_sender, material_filter) = listen(lowpole_hz(20000.0));
    net.chain(Box::new(material_filter));
    // Stereo effects
    let output_node = net.chain(Box::new(
        (pass() * var(&left_amp)) ^ (pass() * var(&right_amp)),
    ));
    println!(
        "Output node: {:?}",
        (sine() >> (pass() * var(&left_amp)) ^ (pass() * var(&right_amp))).outputs()
    );
    net.connect_input(0, input_node, 0);
    net.connect_output(output_node, 0, 0);
    net.connect_output(output_node, 1, 1);
    net.check();

    println!("Net checked.");
    let mut backend = net.backend();
    println!("output backend node: {:?}", backend.outputs());
    // Use `assert_no_alloc` to make sure there are no allocations or deallocations in the audio thread.
    let mut next_value = move || assert_no_alloc(|| backend.get_stereo());

    let err_fn = |err| eprintln!("an error occurred on stream: {}", err);

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            write_data(data, channels, &mut next_value)
        },
        err_fn,
        None,
    )?;
    stream.play()?;

    let mut in_room = false;
    let mut room_amplitude = 1.0;
    //let mut updated_source_info = SourceInfo::default();
    loop {
        if let Ok(info) = source_info.try_read() {
            // Distance attenuation.
            let distance = info.relative_position.norm();
            let amp = 1.0 / (1.0 + (distance / 10.0).powi(2));

            // Orientation hears attenuation.
            let uv = info.relative_position.cross(&info.direction);
            let coeff = (uv.norm() / distance) * uv.dot(&-UP_VECTOR).signum();

            left_amp.set_value((1.0 + coeff) / 2.0);
            right_amp.set_value((1.0 - coeff) / 2.0);
            // Room effects.
            if let Some(room) = &info.room {
                if !in_room {
                    in_room = true;
                    room_amplitude = room_amplitude_factor(Some(room.clone()));
                    material_filter_sender
                        .try_send(Setting::center(10.0))
                        .expect("Failed to send setting to material filter.");
                }
            } else if in_room {
                in_room = false;
                room_amplitude = room_amplitude_factor(None);
            }
            println!(" amplitude: {}", amp * room_amplitude);
            print!("left: {}, right: {}", left_amp.value(), right_amp.value());
            amplitude.set_value(amp * room_amplitude);
        }

        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn write_data<T>(output: &mut [T], channels: usize, next_sample: &mut dyn FnMut() -> (f32, f32))
where
    T: SizedSample + FromSample<f32>,
{
    for frame in output.chunks_mut(channels) {
        let sample = next_sample();
        let left = T::from_sample(sample.0);
        let right = T::from_sample(sample.1);

        for (channel, sample) in frame.iter_mut().enumerate() {
            if channel & 1 == 0 {
                *sample = left;
            } else {
                *sample = right;
            }
        }
    }
}
