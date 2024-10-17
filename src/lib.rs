#![allow(clippy::precedence)]

use std::sync::{Arc, RwLock};

use assert_no_alloc::*;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use crossbeam_channel::{Receiver, Sender};
use fundsp::hacker::*;
use nalgebra::Vector3;

#[cfg(debug_assertions)] // required when disable_release is set (default)
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

const SOUND_SPEED: f32 = 343.0;
pub const HEAD_RADIUS: f32 = 0.10;
#[derive(Clone)]
pub struct SourceInfo {
    pub relative_position: Vector3<f32>,
    pub direction: Vector3<f32>,
}

impl Default for SourceInfo {
    fn default() -> Self {
        SourceInfo {
            relative_position: Vector3::new(0.0, 0.0, 0.0),
            direction: Vector3::new(1.0, 0.0, 0.0),
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

pub fn run_out<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    receiver: Receiver<(f32, f32)>,
    source_info: Arc<RwLock<SourceInfo>>,
) -> Result<(), anyhow::Error>
where
    T: SizedSample + FromSample<f32> + Send,
{
    let input = An(InputNode::new(receiver));

    let sample_rate = config.sample_rate.0 as f64;
    let channels = config.channels as usize;
    let amplitude: Shared = shared(1.0);
    let distance_delay = shared(0.0);

    let mut net = Net::wrap(Box::new(input));
    net.set_sample_rate(sample_rate);
    net.chain(Box::new(tick() * var(&amplitude)));
    net.chain(Box::new(lowpass_hz(18000.0, 0.1)));
    net.chain(Box::new(highpass_hz(30.0, 0.1)));

    net.check();
    println!("Net checked.");
    let backend = net.backend();

    let mut backend = BlockRateAdapter::new(Box::new(backend));

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
    let mut updated_source_info = SourceInfo::default();
    loop {
        if let Ok(info) = source_info.try_read() {
            updated_source_info = info.clone();
            let distance = updated_source_info.relative_position.norm();
            let amp = 1.0 / (1.0 + (distance).powi(2));
            println!("Distance: {}, Amplitude: {}", distance, amp);
            amplitude.set_value(amp);
        }

        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    Ok(())
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
