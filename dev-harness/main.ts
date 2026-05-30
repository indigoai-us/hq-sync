import { mount } from 'svelte';
import Harness from './Harness.svelte';
import '../src/styles/popover.css';

mount(Harness, { target: document.getElementById('app')! });
